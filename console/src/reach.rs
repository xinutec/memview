//! Stretches when a client could not reach this console, as the client measured
//! them: from its last answer to its first answer again, handed over once it is
//! back. Nothing here can see them happen — no request arrives — so the client's
//! word is the only record, kept beside the way in's own ([`crate::tunnel`]).
//! A client measures only while it is on screen; in the background its phone
//! freezes it, so a stretch it reports also ends when it went there.

use std::path::PathBuf;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// How many stretches are kept: enough to see a pattern, few enough to read.
pub const KEPT: usize = 100;

/// One stretch without an answer, in the client's epoch milliseconds. It began
/// between `last_answer` and `first_failure` and ended between `last_failure`
/// and `answered_again`: the client asks every few seconds, not continuously.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Unreached {
    /// Absent when the client had heard nothing since it started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub last_answer: Option<i64>,
    pub first_failure: i64,
    pub last_failure: i64,
    pub answered_again: i64,
    /// How many asks went unanswered.
    pub failures: u32,
}

/// Every stretch clients have reported, oldest first.
#[derive(Debug)]
pub struct Reports {
    store: PathBuf,
    held: Mutex<Vec<Unreached>>,
}

impl Reports {
    /// Read what the last run kept; unreadable is said, and started over.
    pub fn load(store: PathBuf) -> Self {
        let held = match std::fs::read(&store) {
            Err(_) => Vec::new(),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(held) => held,
                Err(error) => {
                    tracing::error!(
                        "reach: {} could not be read, so its stretches are lost: {error}",
                        store.display()
                    );
                    Vec::new()
                }
            },
        };
        Self {
            store,
            held: Mutex::new(held),
        }
    }

    /// Keep what a client reports. A stretch it reported already — the answer to
    /// its last report lost on the way — is kept once.
    pub fn add(&self, reported: Vec<Unreached>) {
        let all = {
            let mut held = self.held.lock();
            for one in reported {
                if held.iter().any(|it| it.first_failure == one.first_failure) {
                    continue;
                }
                tracing::warn!(
                    "reach: a client had no answer for {}s, from {} to {}",
                    (one.answered_again - one.first_failure) / 1000,
                    one.first_failure,
                    one.answered_again
                );
                held.push(one);
            }
            held.sort_by_key(|it| it.first_failure);
            let surplus = held.len().saturating_sub(KEPT);
            held.drain(..surplus);
            held.clone()
        };
        match serde_json::to_vec_pretty(&all) {
            Ok(bytes) => {
                if let Err(error) = std::fs::write(&self.store, bytes) {
                    tracing::warn!("reach: could not write {}: {error}", self.store.display());
                }
            }
            Err(error) => tracing::warn!("reach: could not encode the stretches: {error}"),
        }
    }

    pub fn all(&self) -> Vec<Unreached> {
        self.held.lock().clone()
    }
}
