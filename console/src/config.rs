//! What the console is allowed to do, and where it listens.
//!
//! **The default bind address is loopback, and that is a security decision.**
//! The console can start agents that hold this machine's git credentials, its
//! kubeconfig and its tokens, so an unauthenticated listener on the LAN would
//! hand that to every unpatched device in the house. Loopback needs no
//! authentication for the opposite reason: a process already running as this
//! user can spawn `claude` itself and gains nothing by asking us. Binding
//! anywhere else waits for the client-certificate gate — see
//! `docs/agent-console.md`.

use std::path::{Path, PathBuf};

use crate::session::Spawn;

/// Where the gate's own material comes from. Absent means no TLS, which means
/// loopback only.
#[derive(Debug, Clone)]
pub struct Tls {
    pub cert_file: String,
    pub key_file: String,
    /// SHA-256 of each client key's SubjectPublicKeyInfo, as hex.
    pub pins: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    /// A second, plaintext listener for this machine only, used **only** when the
    /// gate is on.
    ///
    /// Turning the gate on would otherwise take the desk console away: the socket
    /// then demands a client certificate from everybody, and the Mac is headless,
    /// so the desk reaches it through an SSH forward that has no certificate to
    /// present. The carve-out is sound for the same reason loopback needs no
    /// authentication at all — a process already running as this user can spawn
    /// `claude` itself. It is a second port rather than the same one because a
    /// wildcard bind already covers loopback.
    pub desk: String,
    /// Set only when a certificate, a key and at least one pin are all present.
    pub tls: Option<Tls>,
    /// Directories a session may be started in, and their subdirectories.
    pub dirs: Vec<PathBuf>,
    pub spawn: Spawn,
    pub static_dir: Option<String>,
}

/// Where sessions may run when nothing says otherwise: the working repositories.
const DEFAULT_DIRS: &str = "Code";

impl Config {
    pub fn from_env() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let dirs = std::env::var("CONSOLE_DIRS")
            .unwrap_or_else(|_| format!("{home}/{DEFAULT_DIRS}"))
            .split(':')
            .filter(|entry| !entry.is_empty())
            .map(PathBuf::from)
            .collect();
        // All three or none: a certificate with no pins would serve TLS to
        // anybody, and pins with no certificate would look configured while the
        // socket stayed plaintext. Both are worse than being plainly off.
        let tls = match (
            std::env::var("CONSOLE_TLS_CERT"),
            std::env::var("CONSOLE_TLS_KEY"),
            std::env::var("CONSOLE_CLIENT_KEYS"),
        ) {
            (Ok(cert_file), Ok(key_file), Ok(pins)) if !pins.trim().is_empty() => Some(Tls {
                cert_file,
                key_file,
                // ⚠ **The file wins over the variable, and that is the whole
                // point.** `console.sh` reads the list once and exports it, and
                // an upgrade is an `execve` — which inherits the environment. So
                // a console upgraded after an enrolment carried the pin list
                // from whenever it was *originally* launched, and refused the
                // phone it had just been told to trust, naming the very key that
                // was sitting in the file. Only a full restart could pick it up,
                // and a full restart is exactly what the handover exists to
                // avoid: it would end every live session to admit one key.
                //
                // Read here rather than fixed in the script because the script
                // cannot fix it: the staleness is in the environment of a
                // process that is already running.
                pins: pinned_clients(&home)
                    .unwrap_or_else(|| pins.split(',').map(|p| p.trim().to_string()).collect()),
            }),
            _ => None,
        };
        Self {
            // 8091 is memview's and 8092 was already taken on this Mac by a
            // python service, which is exactly the kind of thing a default that
            // was picked by counting upwards runs into.
            bind: std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8097".to_string()),
            desk: std::env::var("CONSOLE_DESK_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8096".to_string()),
            tls,
            dirs,
            spawn: Spawn {
                binary: std::env::var("CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string()),
                model: std::env::var("CONSOLE_MODEL")
                    .ok()
                    .filter(|m| !m.is_empty()),
                // Unset means the CLI's own default, under which a headless
                // session is refused every tool call that needs permission —
                // see `Spawn::permission_mode`. Choosing `acceptEdits` is how a
                // phase-1 console gets work done in a directory that is trusted;
                // choosing `bypassPermissions` hands the machine over, and the
                // console will not pick either on anybody's behalf.
                permission_mode: std::env::var("CONSOLE_PERMISSION_MODE")
                    .ok()
                    .filter(|mode| !mode.is_empty()),
            },
            static_dir: std::env::var("STATIC_DIR").ok(),
        }
    }

    /// The repositories a session could sensibly be started in.
    ///
    /// One level down from each allowed directory, keeping what has a `.git` —
    /// which for `~/Code` is exactly the list of working repositories. It is a
    /// convenience for the client's picker and not a restriction: `resolve`
    /// still admits any directory inside an allowed one, because work happens in
    /// subdirectories too.
    pub fn repos(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .dirs
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().join(".git").exists())
            .map(|entry| entry.path().display().to_string())
            .collect();
        found.sort();
        found
    }

    /// Resolve a requested directory, or say why it is refused.
    ///
    /// Symlinks are resolved on both sides before comparing, because a symlink
    /// inside an allowed directory pointing out of it is the obvious way past a
    /// prefix check. This is a guard rail rather than a boundary — a session
    /// that does start can reach the whole disk — so it is written to catch
    /// mistakes, not to contain an adversary who already has the console.
    pub fn resolve(&self, requested: &str) -> Result<PathBuf, String> {
        let asked = Path::new(requested);
        let real = asked
            .canonicalize()
            .map_err(|err| format!("{requested}: {err}"))?;
        if !real.is_dir() {
            return Err(format!("{requested} is not a directory"));
        }
        for allowed in &self.dirs {
            let root = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
            if real == root || real.starts_with(&root) {
                return Ok(real);
            }
        }
        Err(format!("{requested} is not inside an allowed directory"))
    }
}

/// The pinned client keys, read from the file a person edits.
///
/// One pin per line, `#` comments and blank lines ignored — the same shape
/// `scripts/console.sh` parses, and the same file `scripts/enrol.sh` appends to.
/// `None` when there is no such file, which is what leaves the environment
/// variable in charge for a deployment that does not keep one.
///
/// Returning `None` for an *empty* file is deliberate: a file that exists but
/// says nothing is far more likely to be a mistake than an instruction to trust
/// nobody, and the failure it would cause — every client refused — looks
/// identical to the certificate being wrong.
fn pinned_clients(home: &str) -> Option<Vec<String>> {
    let dir =
        std::env::var("CONSOLE_HOME").unwrap_or_else(|_| format!("{home}/.config/agent-console"));
    let text = std::fs::read_to_string(PathBuf::from(dir).join("clients")).ok()?;
    let pins: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    (!pins.is_empty()).then_some(pins)
}
