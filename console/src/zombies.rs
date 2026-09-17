//! A `<defunct>` under the console, recorded at the moment it is seen.
//!
//! A zombie's command is gone (`comm` and `command` both read `<defunct>`), so a
//! sighting carries `ppid` and `lstart`; the start time is what pairs it with the
//! `asking pid N` lines the spawn sites log, safely across pid reuse.
//!
//! This reads the process table and must never reap: `SIGCHLD` as `SIG_IGN` or
//! `waitpid(-1, …)` would take the exit status [`crate::session::Session::reap`] reads.

use std::collections::HashSet;

/// One `<defunct>` child, as much of it as survives. No command field: a recorded
/// `"<defunct>"` would read like a fact.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sighting {
    pub pid: u32,
    /// `lstart`, verbatim as `ps` prints it — the pairing key, not a timestamp
    /// to compute with.
    pub started: String,
}

/// The zombies whose parent is `parent`, from a `ps` table of `pid ppid state
/// lstart` — four columns, the last of them five words. Shorter rows are skipped.
pub fn parse(table: &str, parent: u32) -> Vec<Sighting> {
    let mut out = Vec::new();
    for row in table.lines() {
        let mut word = row.split_whitespace();
        let (Some(pid), Some(ppid), Some(state)) = (word.next(), word.next(), word.next()) else {
            continue;
        };
        // `Z` and `Z+` are both zombies; the suffix is job-control state.
        if !state.starts_with('Z') {
            continue;
        }
        let (Ok(pid), Ok(ppid)) = (pid.parse::<u32>(), ppid.parse::<u32>()) else {
            continue;
        };
        if ppid != parent {
            continue;
        }
        let started = word.collect::<Vec<_>>().join(" ");
        if started.is_empty() {
            continue;
        }
        out.push(Sighting { pid, started });
    }
    out
}

/// What has already been reported, so each zombie is logged once and not once a
/// minute for as long as it lasts.
#[derive(Default)]
pub struct Watch {
    seen: HashSet<Sighting>,
}

impl Watch {
    /// The sightings new since the last sweep, and the ones gone. Departures matter:
    /// a zombie still there an hour later is a leak, one that disappears is late reaping.
    pub fn sweep(&mut self, table: &str, parent: u32) -> (Vec<Sighting>, Vec<Sighting>) {
        let now: HashSet<Sighting> = parse(table, parent).into_iter().collect();
        let fresh = now.difference(&self.seen).cloned().collect();
        let gone = self.seen.difference(&now).cloned().collect();
        self.seen = now;
        (fresh, gone)
    }
}

/// Ask the process table, once.
fn table() -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid=,state=,lstart="])
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Log every `<defunct>` under this process as it appears, and as it goes. A
/// minute apart — often enough to catch one that something reaps late.
pub async fn watch() {
    let parent = std::process::id();
    let mut watch = Watch::default();
    loop {
        if let Some(table) = table() {
            let (fresh, gone) = watch.sweep(&table, parent);
            for zombie in fresh {
                // Beside `gists: asking pid N` and `deaf: asking pid N`, which
                // is what this line is for.
                tracing::warn!(
                    "zombies: <defunct> pid {} under this console, started {} \
                     — match the pid against an `asking pid` line above",
                    zombie.pid,
                    zombie.started
                );
            }
            for zombie in gone {
                tracing::info!(
                    "zombies: pid {} is gone — reaped late, not leaked",
                    zombie.pid
                );
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}
