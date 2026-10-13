//! Signing the account in again from the phone. Every session shares one login,
//! and when it lapses the cure was a terminal on the Mac. `claude auth login`
//! needs none: it prints a link whose page shows a code, and waits for the code on
//! its stdin. The console runs it, hands the link to the phone, and writes back
//! the code the phone sends. The code is never logged or kept: it lives in this
//! process until the CLI has read it.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt as _, AsyncWriteExt as _};
use tokio::process::{Child, ChildStdin};

/// How long the CLI may take to print its link.
const LINK_WITHIN: Duration = Duration::from_secs(30);
/// How long it may take to exchange a code once it has one.
const EXCHANGED_WITHIN: Duration = Duration::from_secs(120);

/// The one sign-in under way, if any. One at a time: the login is the account's,
/// and a second start replaces the first, whose process is killed with it.
#[derive(Default)]
pub struct Login(tokio::sync::Mutex<Option<Attempt>>);

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

/// The code the sign-in page showed. No `Debug`, so it cannot be logged by
/// accident.
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
        let mut attempt = self.0.lock().await;
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
        // Both streams: which one the link is printed on is the CLI's business.
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
            .0
            .lock()
            .await
            .take()
            .ok_or("no sign-in is waiting for a code")?;
        // What came before is the link and the prompt; only the answer is news.
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
        Ok(LoginResult {
            ok: status.success(),
            said,
        })
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

/// The sign-in link in what the CLI has said so far: a whole URL, so only once
/// the line it is on has ended.
fn link(said: &str) -> Option<String> {
    said.lines()
        .take(said.matches('\n').count())
        .flat_map(str::split_whitespace)
        .find(|word| word.starts_with("https://") && word.contains("/oauth/authorize"))
        .map(str::to_string)
}
