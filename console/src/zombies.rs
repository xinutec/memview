//! A `<defunct>` under the console, recorded at the moment it is seen.
//!
//! A sighting carries `ppid` and `lstart`, which pair it with the spawn sites'
//! `asking pid N` lines. Never reaps: that would take exit statuses
//! [`crate::session::Session::reap`] needs.

use std::collections::HashSet;

/// One `<defunct>` child, as much of it as survives.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sighting {
    pub pid: u32,
    /// `lstart`, verbatim — the pairing key.
    pub started: String,
}

/// The zombies whose parent is `parent`, from a `ps` table of `pid ppid state lstart`.
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

/// What has been reported, so each zombie is logged once.
#[derive(Default)]
pub struct Watch {
    seen: HashSet<Sighting>,
}

impl Watch {
    /// The sightings new since the last sweep, and the ones gone.
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

/// Log every `<defunct>` under this process as it appears and goes, once a minute.
pub async fn watch() {
    let parent = std::process::id();
    let mut watch = Watch::default();
    loop {
        if let Some(table) = table() {
            let (fresh, gone) = watch.sweep(&table, parent);
            for zombie in fresh {
                // Pairs with the spawn sites' `asking pid N`.
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
