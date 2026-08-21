//! A `<defunct>` under the console, recorded at the moment it is seen.
//!
//! #797 is a zombie whose parent is the console and whose origin nothing
//! recorded. Every look so far has been after the fact at a process table that
//! has already been swept, and waiting has now failed twice — 0 sightings in the
//! nine days to 2026-08-21, across a window in which neither suspect's failing
//! path ran. So the occurrence has to carry its own evidence.
//!
//! ⚠ **A zombie's command is gone, so the occurrence cannot say what it was.**
//! Measured on this Mac 2026-08-21 by forcing one: a process in state `Z` still
//! reports `ppid`, `lstart` and `etime`, but **both `comm` and `command` read
//! `<defunct>`**. The task asked for "ppid and the command"; only the first half
//! exists.
//!
//! **The start time is the half that identifies it.** `gist.rs` and `deaf.rs`
//! already log `asking pid N` at their spawn, so a sighting pairs with a spawn
//! site by pid — and `lstart` is what makes that pairing safe across pid reuse,
//! which a bare pid on a host up for weeks cannot promise.
//!
//! ⚠ **This reads the process table; it must never reap.** `SIGCHLD` set to
//! `SIG_IGN` and `waitpid(-1, …)` both reap indiscriminately and would take the
//! exit status [`crate::session::Session::reap`] reads. `ps` takes nothing.

use std::collections::HashSet;

/// One `<defunct>` child, as much of it as survives.
///
/// No command field, deliberately: see the module note. A `Sighting` that
/// carried `"<defunct>"` would read like a recorded fact rather than an absent
/// one.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sighting {
    pub pid: u32,
    /// `lstart`, verbatim as `ps` prints it — the pairing key, not a timestamp
    /// to compute with.
    pub started: String,
}

/// The zombies whose parent is `parent`, from a `ps` table.
///
/// Expects rows of `pid ppid state lstart`, which is four columns of which the
/// last is five words. Anything shorter is a row this does not understand, and
/// is skipped rather than guessed at.
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
    /// The sightings that are new since the last sweep, and the ones that have
    /// gone.
    ///
    /// **Departures are worth a line too.** A zombie that is still there an hour
    /// later is a leak; one that disappears is something reaping late, and the
    /// two want different fixes. Nothing else records which of those is
    /// happening.
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

/// Log every `<defunct>` under this process as it appears, and as it goes.
///
/// A minute apart. The zombies this hunts have persisted for days — the count
/// this task was filed on went 22 → 23 and stayed — so the cadence is not what
/// decides whether one is caught. It is set for the case that would otherwise
/// leave nothing at all: a zombie that something reaps late, which a slow sweep
/// would miss entirely and report as "none, still".
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
