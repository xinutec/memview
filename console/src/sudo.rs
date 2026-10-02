//! A command run as root on a password typed on the phone. Claude asks through
//! `agent-console sudo`; the console shows the command and a password field, and
//! runs `sudo` itself with the password on its stdin. The password never passes
//! through anything the session can read, and is never logged or kept: it lives
//! in this process until `sudo` has read it.

use std::collections::BTreeMap;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::oneshot;

/// Requests waiting for their password, by ask id. `None` is a refusal.
#[derive(Default)]
pub struct Waiting(Mutex<BTreeMap<String, oneshot::Sender<Option<String>>>>);

impl Waiting {
    pub fn wait(&self, id: &str) -> oneshot::Receiver<Option<String>> {
        let (tx, rx) = oneshot::channel();
        self.0.lock().insert(id.to_string(), tx);
        rx
    }

    /// Hand the password over. False when nothing waits for it any more.
    pub fn answer(&self, id: &str, password: Option<String>) -> bool {
        match self.0.lock().remove(id) {
            Some(tx) => tx.send(password).is_ok(),
            None => false,
        }
    }
}

/// What `agent-console sudo` sends.
#[derive(Serialize, Deserialize)]
pub struct Request {
    pub session: String,
    pub cwd: String,
    pub command: String,
}

/// What it prints and exits with.
#[derive(Serialize, Deserialize)]
pub struct Ran {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The phone's answer: a password to run it with, or none to refuse. No
/// `Debug`, so it cannot be logged by accident.
#[derive(Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SudoAnswer {
    pub id: String,
    pub password: Option<String>,
}

/// Run `command` under bash as root. `-k`: no cached login is used or left
/// behind, so nothing else the session runs gains root from this.
pub async fn run(cwd: &str, command: &str, password: String) -> Ran {
    let spawned = tokio::process::Command::new("/usr/bin/sudo")
        .args(["-k", "-S", "-p", "", "--", "/bin/bash", "-c", command])
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(child) => child,
        Err(why) => return refused(&format!("sudo could not start: {why}")),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(format!("{password}\n").as_bytes()).await;
    }
    drop(password);
    match child.wait_with_output().await {
        Ok(out) => Ran {
            status: out.status.code().unwrap_or(1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(why) => refused(&format!("sudo failed: {why}")),
    }
}

pub fn refused(why: &str) -> Ran {
    Ran {
        status: 1,
        stdout: String::new(),
        stderr: format!("{why}\n"),
    }
}
