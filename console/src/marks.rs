//! The landmarks of each transcript, walked once and then only extended.
//!
//! Correct because transcripts are append-only; a file that shrank is re-walked.

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

    /// Every landmark in this transcript, walking only what is new. Blocking — keep it
    /// off the executor. The lock is not held across the walk.
    pub fn of(&self, id: &str, path: &Path) -> Vec<Landmark> {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let known = self.held.read().get(id).cloned().unwrap_or_default();

        // Nothing complete has arrived since the last walk.
        if len == known.through {
            return known.found;
        }
        // The file shrank: start again.
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

    /// Forget a conversation this console no longer holds.
    pub fn forget(&self, id: &str) {
        self.held.write().remove(id);
    }
}
