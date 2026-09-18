//! The landmarks of each transcript, walked once and then only extended.
//!
//! The walk is the wait, not the payload, so only an incremental walk moves it.
//! Append-only is what makes the cache correct — [`crate::past::counted`] already
//! trusts everything before a stored offset — and a file that SHRANK (compaction
//! rewrites history) is re-walked, not extended.

use parking_lot::RwLock;
use std::collections::BTreeMap;
use std::path::Path;

use crate::past::Landmark;

/// What one transcript's walk found, and how far it got.
#[derive(Debug, Clone, Default)]
struct Walked {
    found: Vec<Landmark>,
    /// The byte the next walk starts at — the length of the file as it was read.
    through: u64,
}

/// Every transcript's landmarks, by session id.
#[derive(Debug, Default)]
pub struct Marks {
    held: RwLock<BTreeMap<String, Walked>>,
}

impl Marks {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every landmark in this transcript, walking only what has arrived since last time.
    ///
    /// Blocking, and it must stay off the executor: the first call pays the whole walk,
    /// seconds on a large file. The lock is not held across it — two requests for the
    /// same conversation both walk and the later answer wins, rather than every other
    /// sheet queueing behind this one.
    pub fn of(&self, id: &str, path: &Path) -> Vec<Landmark> {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let known = self.held.read().get(id).cloned().unwrap_or_default();

        // Nothing complete has arrived since the last walk. `through` is where the walk
        // STOPPED, so a half-written tail line is re-read until it is finished — bounded
        // by one line.
        if len == known.through {
            return known.found;
        }
        // The file went backwards, so every offset kept here may name a byte in
        // a different line. Start again rather than splice two histories.
        let from = if len < known.through {
            0
        } else {
            known.through
        };
        let mut found = if from == 0 { Vec::new() } else { known.found };

        let began = std::time::Instant::now();
        let walk = crate::past::landmarks_from(path, from);
        found.extend(walk.found);
        tracing::debug!(
            "{id}: {} landmark(s), {} from byte {from}, in {:?}",
            found.len(),
            walk.through.saturating_sub(from),
            began.elapsed()
        );

        self.held.write().insert(
            id.to_string(),
            Walked {
                found: found.clone(),
                through: walk.through,
            },
        );
        found
    }

    /// Forget a conversation this console no longer holds; otherwise the map only
    /// grows, a Vec per conversation.
    pub fn forget(&self, id: &str) {
        self.held.write().remove(id);
    }
}
