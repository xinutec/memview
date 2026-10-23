//! Signing the shared login in again from the phone: the console runs `claude auth
//! login`, hands its link to the phone, and writes back the code the link's page
//! shows. The code is never logged or kept. Also when the login lapses, for a
//! warning before it does: [`Login::lapses_at`].

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child, ChildStdin};

/// How long the CLI may take to print its link.
const LINK_WITHIN: Duration = Duration::from_secs(30);
/// How long it may take to exchange a code once it has one.
const EXCHANGED_WITHIN: Duration = Duration::from_secs(120);
/// How long a read of the lapse is reused: it moves only on a sign-in, which
/// forgets it at once.
const LAPSE_KEPT: Duration = Duration::from_secs(600);
/// The Keychain item the CLI keeps its login in.
const CREDENTIALS: &str = "Claude Code-credentials";

/// The one sign-in under way, if any — a second start replaces the first — and
/// the last read of when the login lapses.
#[derive(Default)]
pub struct Login {
    attempt: tokio::sync::Mutex<Option<Attempt>>,
    lapse: parking_lot::Mutex<Option<(std::time::Instant, Option<i64>)>>,
}

/// When the login lapses, in epoch milliseconds: the renewal token's fixed expiry,
/// out of the Keychain item's JSON. Seconds are read as seconds. Only that field
/// is parsed; the tokens beside it are never kept or logged.
pub fn lapse_of(credentials: &str) -> Option<i64> {
    #[derive(Deserialize)]
    struct Item {
        #[serde(rename = "claudeAiOauth")]
        oauth: Oauth,
    }
    #[derive(Deserialize)]
    struct Oauth {
        #[serde(rename = "refreshTokenExpiresAt")]
        lapses: Option<i64>,
    }
    let at = serde_json::from_str::<Item>(credentials)
        .ok()?
        .oauth
        .lapses?;
    Some(if at < 1_000_000_000_000 {
        at * 1000
    } else {
        at
    })
}

struct Attempt {
    child: Child,
    stdin: ChildStdin,
    said: Arc<parking_lot::Mutex<String>>,
}

/// Where to sign in.
#[derive(Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct LoginStarted {
    pub url: String,
}

/// The code the sign-in page showed. No `Debug`, so it cannot be logged.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct LoginCode {
    pub code: String,
}

/// How the exchange went, in the CLI's own words.
#[derive(Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct LoginResult {
    pub ok: bool,
    pub said: String,
}

impl Login {
    /// Run `<binary> auth login` and wait for the link it prints.
    pub async fn start(&self, binary: &str) -> Result<LoginStarted, String> {
        let mut attempt = self.attempt.lock().await;
        // The previous attempt's process goes with it: `kill_on_drop`.
        *attempt = None;
        let mut child = tokio::process::Command::new(binary)
            .args(["auth", "login"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|why| format!("could not start {binary} auth login: {why}"))?;
        let said = Arc::new(parking_lot::Mutex::new(String::new()));
        // Both streams: the link may be on either.
        if let Some(out) = child.stdout.take() {
            tokio::spawn(collect(out, Arc::clone(&said)));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(collect(err, Arc::clone(&said)));
        }
        let stdin = child.stdin.take().ok_or("no stdin to write the code to")?;
        let deadline = tokio::time::Instant::now() + LINK_WITHIN;
        loop {
            let found = link(&said.lock());
            if let Some(url) = found {
                *attempt = Some(Attempt { child, stdin, said });
                return Ok(LoginStarted { url });
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!(
                    "auth login ended ({status}) with no link: {}",
                    said.lock().trim()
                ));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "no link from auth login within {}s",
                    LINK_WITHIN.as_secs()
                ));
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Hand the code over and wait for the exchange.
    pub async fn finish(&self, code: LoginCode) -> Result<LoginResult, String> {
        let Attempt {
            mut child,
            mut stdin,
            said,
        } = self
            .attempt
            .lock()
            .await
            .take()
            .ok_or("no sign-in is waiting for a code")?;
        // Only what follows the code.
        let before = said.lock().len();
        stdin
            .write_all(format!("{}\n", code.code.trim()).as_bytes())
            .await
            .map_err(|why| format!("could not hand the code over: {why}"))?;
        drop(code);
        drop(stdin);
        let status = tokio::time::timeout(EXCHANGED_WITHIN, child.wait())
            .await
            .map_err(|_| format!("no answer within {}s", EXCHANGED_WITHIN.as_secs()))?
            .map_err(|why| format!("auth login: {why}"))?;
        // The readers may still be draining what the process wrote last.
        tokio::time::sleep(Duration::from_millis(100)).await;
        let said = said.lock()[before..].trim().to_string();
        if status.success() {
            *self.lapse.lock() = None;
        }
        Ok(LoginResult {
            ok: status.success(),
            said,
        })
    }
}

impl Login {
    /// When the login lapses, read from the Keychain at most every [`LAPSE_KEPT`]
    /// and again after a sign-in. `None` when it cannot be read.
    pub async fn lapses_at(&self) -> Option<i64> {
        if let Some((read, at)) = *self.lapse.lock()
            && read.elapsed() < LAPSE_KEPT
        {
            return at;
        }
        let read = tokio::time::timeout(
            Duration::from_secs(5),
            tokio::process::Command::new("security")
                .args(["find-generic-password", "-s", CREDENTIALS, "-w"])
                .kill_on_drop(true)
                .output(),
        )
        .await;
        let at = match read {
            Ok(Ok(out)) if out.status.success() => lapse_of(&String::from_utf8_lossy(&out.stdout)),
            _ => None,
        };
        *self.lapse.lock() = Some((std::time::Instant::now(), at));
        at
    }
}

/// Append everything a stream says, as it says it.
async fn collect(mut from: impl AsyncRead + Unpin, into: Arc<parking_lot::Mutex<String>>) {
    let mut buffer = [0u8; 4096];
    while let Ok(read) = from.read(&mut buffer).await {
        if read == 0 {
            break;
        }
        into.lock()
            .push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
}

/// The sign-in link, once its line has ended.
fn link(said: &str) -> Option<String> {
    said.lines()
        .take(said.matches('\n').count())
        .flat_map(str::split_whitespace)
        .find(|word| word.starts_with("https://") && word.contains("/oauth/authorize"))
        .map(str::to_string)
}
