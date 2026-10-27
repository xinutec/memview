//! The phone's way in: an SSH connection this Mac dials to isis, which listens on
//! its VPN address and passes what arrives back down to the console on loopback.
//!
//! Nothing connects toward this Mac, so the one-way VPN rule stands. isis carries
//! only ciphertext: TLS ends at the console's own gate, so isis can drop the way in
//! but not read, inject or impersonate. The key may do one thing
//! (`restrict,port-forwarding,permitlisten=…` in nixos-config's isis configuration).
//!
//! The ssh is the console's child, so it ends with the console: a way in with
//! nothing behind it is an open port on isis. An upgrade carries it across like a
//! session, so the phone does not lose it to one.

#![expect(
    unsafe_code,
    reason = "libc process control (waitpid, kill, a raw descriptor) for the ssh it supervises across an upgrade"
)]

use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, BufReader};

/// Where an upgrade hands the running ssh to the next image.
pub const HANDOVER: &str = "CONSOLE_TUNNEL_HANDOVER";

/// Must match nixos-config's network.nix: the address isis listens on for the phone.
pub const LISTEN: &str = "10.100.0.2:8097";

/// Between a dial ending and the next. With [`CONNECT_TIMEOUT`] a down isis is
/// tried every 15 seconds, so the way in is back within that of isis.
const REDIAL: Duration = Duration::from_secs(5);

/// macOS waits about 75 seconds on a host that does not answer, which is how long
/// the way in stayed down after isis was back from a reboot.
const CONNECT_TIMEOUT: &str = "10";

/// How many outages are kept: enough to see a pattern, few enough to read.
const KEPT: usize = 20;

/// How to dial.
#[derive(Debug, Clone)]
pub struct Dial {
    pub program: String,
    pub host: String,
    pub key: PathBuf,
    /// The address on isis the phone connects to.
    pub listen: String,
    /// The console's gated socket, where isis's connections arrive.
    pub local: String,
    /// Where [`WayIn`] is kept, so outages outlive the console.
    pub record: PathBuf,
}

impl Dial {
    /// The way in to the console listening on `local`, as configured, its record
    /// kept in `dir`.
    pub fn from_env(local: &str, dir: &std::path::Path) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let user = std::env::var("USER").unwrap_or_default();
        Self {
            program: "ssh".to_string(),
            host: std::env::var("CONSOLE_TUNNEL_HOST")
                .unwrap_or_else(|_| format!("{user}@isis.xinutec.org")),
            key: std::env::var("CONSOLE_TUNNEL_KEY")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from(home).join(".ssh/console-tunnel")),
            listen: std::env::var("CONSOLE_TUNNEL_LISTEN").unwrap_or_else(|_| LISTEN.to_string()),
            local: local.to_string(),
            record: dir.join("way-in.json"),
        }
    }

    /// ssh's arguments.
    pub fn args(&self) -> Vec<String> {
        [
            "-N",
            // For the line saying isis has the address: see [`heard`].
            "-v",
            "-i",
            &self.key.display().to_string(),
            "-o",
            "IdentitiesOnly=yes",
            // Nobody is there to answer a prompt.
            "-o",
            "BatchMode=yes",
            // Connected but not listening would look healthy and pass nothing.
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            &format!("ConnectTimeout={CONNECT_TIMEOUT}"),
            // A half-open connection keeps isis's port held: 30 s of silence and
            // this end gives up first.
            "-o",
            "ServerAliveInterval=10",
            "-o",
            "ServerAliveCountMax=3",
            "-R",
            &format!("{}:{}", self.listen, self.local),
            &self.host,
        ]
        .map(str::to_string)
        .to_vec()
    }
}

/// What one line of ssh's `-v` output says about the way in.
#[derive(Debug, PartialEq)]
pub enum Heard {
    /// isis has the address and passes connections on.
    Up,
    /// Something went wrong; the words are the cause.
    Trouble(String),
    /// Nothing about the way in.
    Chatter,
}

/// Lines ssh prints without `debug` that are not trouble.
const FINE: [&str; 3] = ["Authenticated to ", "Transferred: ", "Bytes per second: "];

pub fn heard(line: &str) -> Heard {
    let line = line.trim();
    if line.starts_with("debug1: forwarding_success") {
        Heard::Up
    } else if line.is_empty()
        || line.starts_with("debug")
        || FINE.iter().any(|fine| line.starts_with(fine))
    {
        Heard::Chatter
    } else {
        Heard::Trouble(line.to_string())
    }
}

/// A stretch when the phone could not reach the console through isis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Outage {
    /// In epoch milliseconds.
    pub from: i64,
    pub to: i64,
    /// What ssh said as it went down, rather than while it stayed down.
    pub cause: String,
}

/// Whether the way in is up, and the outages it has had.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct WayIn {
    /// Since when isis has passed connections on, in epoch milliseconds.
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_since: Option<i64>,
    /// Since when it has not. Both absent is a first dial still under way.
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down_since: Option<i64>,
    /// Why, for the outage under way.
    #[cfg_attr(feature = "ts", ts(optional))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cause: Option<String>,
    /// Oldest first, at most [`KEPT`].
    #[serde(default)]
    pub outages: Vec<Outage>,
}

impl WayIn {
    /// The record a new console starts from: the outages, and nothing about a
    /// dial, since the last console's ssh ended with it.
    pub fn restarted(self) -> Self {
        Self {
            outages: self.outages,
            ..Self::default()
        }
    }

    /// isis has the address again. The outage it ends, if there was one.
    pub fn up(&mut self, now: i64) -> Option<Outage> {
        let cause = self.cause.take();
        self.up_since = Some(now);
        let from = self.down_since.take()?;
        let outage = Outage {
            from,
            to: now,
            cause: cause.unwrap_or_else(|| "ssh said nothing".to_string()),
        };
        self.outages.push(outage.clone());
        let surplus = self.outages.len().saturating_sub(KEPT);
        self.outages.drain(..surplus);
        Some(outage)
    }

    /// ssh said something went wrong. While up, the newest word: what it says as
    /// it gives up is the cause, a warning hours before is not. While down, the
    /// first: why it went down, not why it stayed down.
    pub fn trouble(&mut self, said: &str) {
        if self.up_since.is_some() {
            self.cause = Some(said.to_string());
        } else {
            self.cause.get_or_insert_with(|| said.to_string());
        }
    }

    /// The ssh ended, with `how` for when it said nothing first.
    pub fn ended(&mut self, now: i64, how: &str) {
        if self.up_since.take().is_some() || self.down_since.is_none() {
            self.down_since = Some(now);
        }
        self.cause.get_or_insert_with(|| how.to_string());
    }
}

/// The ssh an upgrade hands on: still this pid's child, as `execve` keeps
/// parentage, with its stderr kept open across the exec.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Running {
    pub pid: u32,
    pub stderr: RawFd,
}

/// The way in, kept up for as long as the console runs.
pub struct Tunnel {
    dial: Dial,
    way: Mutex<WayIn>,
    running: Mutex<Option<Running>>,
}

impl Tunnel {
    pub fn new(dial: Dial) -> Arc<Self> {
        // No record is a first start; an unreadable one is said, and started over.
        let kept = match std::fs::read(&dial.record) {
            Err(_) => WayIn::default(),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(kept) => kept,
                Err(error) => {
                    tracing::error!(
                        "way in: {} could not be read, so its outages are lost: {error}",
                        dial.record.display()
                    );
                    WayIn::default()
                }
            },
        };
        Arc::new(Self {
            dial,
            way: Mutex::new(kept),
            running: Mutex::new(None),
        })
    }

    pub fn way_in(&self) -> WayIn {
        self.way.lock().clone()
    }

    /// Dial, or take over the ssh the previous image handed on, and redial for as
    /// long as the console runs.
    pub fn start(self: &Arc<Self>) {
        let carried = std::env::var(HANDOVER)
            .ok()
            .and_then(|handed| serde_json::from_str::<Running>(&handed).ok());
        // SAFETY: at startup, before anything reads the environment concurrently.
        // So a later image does not adopt a pid that is long gone.
        unsafe { std::env::remove_var(HANDOVER) };
        if carried.is_none() {
            let mut way = self.way.lock();
            *way = std::mem::take(&mut *way).restarted();
        }
        let tunnel = self.clone();
        tokio::spawn(async move { tunnel.keep_up(carried).await });
    }

    async fn keep_up(self: Arc<Self>, mut carried: Option<Running>) {
        loop {
            let how = match carried.take() {
                Some(running) => {
                    tracing::info!("way in: carried ssh {} across the upgrade", running.pid);
                    self.adopted(running).await
                }
                None => {
                    tracing::info!(
                        "way in: dialling {} for {}",
                        self.dial.host,
                        self.dial.listen
                    );
                    self.dialled().await
                }
            };
            *self.running.lock() = None;
            let cause = {
                let mut way = self.way.lock();
                way.ended(now(), &how);
                way.cause.clone().unwrap_or(how)
            };
            self.keep();
            tracing::warn!(
                "way in: down ({cause}), redialling in {}s",
                REDIAL.as_secs()
            );
            tokio::time::sleep(REDIAL).await;
        }
    }

    /// One ssh from start to end; how it ended.
    async fn dialled(&self) -> String {
        let mut child = match tokio::process::Command::new(&self.dial.program)
            .args(self.dial.args())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) => return format!("{} could not be started: {error}", self.dial.program),
        };
        if let (Some(pid), Some(stderr)) = (child.id(), child.stderr.take()) {
            *self.running.lock() = Some(Running {
                pid,
                stderr: stderr.as_raw_fd(),
            });
            self.listen(stderr).await;
        }
        match child.wait().await {
            Ok(status) => format!("ssh {status}"),
            Err(error) => format!("ssh could not be waited for: {error}"),
        }
    }

    /// The ssh the previous image started, to its end.
    async fn adopted(&self, running: Running) -> String {
        // SAFETY: the previous image handed this descriptor on, with close-on-exec
        // cleared so it would survive; nothing else here holds it.
        let stderr = unsafe { OwnedFd::from_raw_fd(running.stderr) };
        match tokio::net::unix::pipe::Receiver::from_owned_fd(stderr) {
            Ok(stderr) => {
                *self.running.lock() = Some(running);
                self.listen(stderr).await;
            }
            Err(error) => tracing::error!("way in: the carried ssh's output is lost: {error}"),
        }
        let pid = running.pid;
        tokio::task::spawn_blocking(move || exit_of(pid))
            .await
            .unwrap_or_else(|error| format!("ssh could not be waited for: {error}"))
    }

    /// Read ssh's output to its end.
    async fn listen(&self, output: impl AsyncRead + Unpin) {
        let mut lines = BufReader::new(output).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            match heard(&line) {
                Heard::Up => {
                    let ended = self.way.lock().up(now());
                    self.keep();
                    match ended {
                        Some(outage) => tracing::info!(
                            "way in: up, after {}s down ({})",
                            (outage.to - outage.from) / 1000,
                            outage.cause
                        ),
                        None => tracing::info!("way in: up"),
                    }
                }
                Heard::Trouble(said) => {
                    tracing::warn!("way in: ssh: {said}");
                    self.way.lock().trouble(&said);
                }
                Heard::Chatter => {}
            }
        }
    }

    /// Write [`WayIn`] down, so a restart still knows its outages.
    fn keep(&self) {
        let way = self.way.lock().clone();
        let written = serde_json::to_vec(&way)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(std::fs::write(&self.dial.record, bytes)?));
        if let Err(error) = written {
            tracing::warn!(
                "way in: could not keep its record in {}: {error}",
                self.dial.record.display()
            );
        }
    }

    /// The running ssh for the next image, its stderr kept open across the exec.
    /// `None` when there is none, or its output could not be kept.
    pub fn hand_over(&self) -> Option<(&'static str, String)> {
        let running = (*self.running.lock())?;
        if !crate::session::keepable(running.stderr) {
            return None;
        }
        serde_json::to_string(&running)
            .ok()
            .map(|handed| (HANDOVER, handed))
    }

    /// End the ssh with the console, so isis is not left listening for nothing.
    pub fn stop(&self) {
        if let Some(running) = *self.running.lock() {
            // SAFETY: kill only sends a signal; the pid is our child.
            unsafe { libc::kill(running.pid as libc::pid_t, libc::SIGTERM) };
        }
    }
}

/// Wait for an adopted child; tokio cannot, as it never spawned it.
fn exit_of(pid: u32) -> String {
    let mut status = 0;
    // SAFETY: `pid` is our child (`execve` keeps parentage); waitpid only reads.
    if unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) } < 0 {
        return format!("ssh {pid} could not be waited for");
    }
    if libc::WIFEXITED(status) {
        format!("ssh exit status: {}", libc::WEXITSTATUS(status))
    } else if libc::WIFSIGNALED(status) {
        format!("ssh signal: {}", libc::WTERMSIG(status))
    } else {
        format!("ssh ended ({status})")
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or_default()
}
