//! The landmarks of each transcript, walked once and then only extended.
//!
//! ⚠ **The walk is the wait, not the payload.** On a large conversation the
//! transfer is a rounding error against the walk, and remains the smaller half
//! even over a slow phone link — so sending less would leave the wait where it
//! is. Making the walk incremental is the only thing that moves it.
//!
//! **Append-only is what makes the cache correct**, and it is not an assumption
//! invented here: [`crate::past::counted`] already trusts everything before a
//! stored byte offset on every turn. Landmarks carry absolute offsets, so one
//! found in the first megabyte stays true however much is appended.
//!
//! ⚠ **A file that SHRANK is re-walked, not extended.** Compaction rewrites
//! history, so "smaller than last time" means the offsets here describe a file
//! that no longer exists. Cheap to detect, silently wrong if it is not.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::RwLock;

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

    /// Every landmark in this transcript, walking only what has arrived since
    /// last time.
    ///
    /// ⚠ **Blocking, and it must stay off the executor.** The first call for a
    /// conversation pays the whole walk — seconds on a large file — and no gate
    /// ahead of the parser survives contact with the format
    /// ([`crate::past::landmarks`] records the two that were tried). What this
    /// removes is paying it *again*.
    ///
    /// The lock is not held across the walk. Two requests for the same
    /// conversation arriving together will both walk, and the later answer wins;
    /// duplicating a rare few seconds of work is better than making every other
    /// session's sheet queue behind this one.
    pub fn of(&self, id: &str, path: &Path) -> Vec<Landmark> {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let known = self
            .held
            .read()
            .expect("marks poisoned")
            .get(id)
            .cloned()
            .unwrap_or_default();

        // Nothing complete has arrived since the last walk. The common case once
        // a sheet has been opened, and the whole point of the file.
        //
        // ⚠ Compared against the length, but `through` is where the WALK stopped
        // — so a file whose tail is a half-written line is re-read from that
        // line every time until it is finished. That is the correct amount of
        // work, and it is bounded by one line.
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

        self.held.write().expect("marks poisoned").insert(
            id.to_string(),
            Walked {
                found: found.clone(),
                through: walk.through,
            },
        );
        found
    }

    /// Forget a conversation, when it is no longer one this console holds.
    ///
    /// Without this the map is the one thing here that only grows, and it holds
    /// a Vec per conversation — 6,107 landmarks on the largest, which is the
    /// same 700 kB the wire was carrying.
    pub fn forget(&self, id: &str) {
        self.held.write().expect("marks poisoned").remove(id);
    }
}
