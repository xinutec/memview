//! The landmarks of each transcript, walked once and then only extended.
//!
//! ⚠ **The walk is the whole of the wait, and the task this came from assumed it
//! was the payload.** Measured against the live console 2026-08-15, on the
//! biggest conversation here:
//!
//! ```text
//! 6.019 s   the walk, server-side          (console's own log line)
//! 6.023 s   the whole request, loopback
//!   701 kB  the answer
//! ```
//!
//! So the transfer is four milliseconds of it. memview #808 reads *"the walk is
//! slow once, the payload is slow every time and on the worst connection"* and
//! puts the payload first — but both happen on every open, and even over a phone
//! link slow enough to spend three seconds on 701 kB the walk is still the larger
//! half. Sending less would have left a six-second progress bar exactly where it
//! was.
//!
//! **What makes a cache correct here is append-only, which is not an assumption
//! this file invented.** [`crate::past::counted`] already reads each transcript
//! from a stored byte offset and trusts everything before it, every turn, for
//! every live session. Landmarks carry absolute offsets from the start of the
//! file, so a landmark found in the first megabyte stays true however much is
//! appended after it.
//!
//! ⚠ **A file that SHRANK is not extended, it is re-walked.** The transcript is
//! rewritten in places — compaction rewrites history, and roughly a fifth of a
//! big file is second copies of lines — so "smaller than last time" means the
//! ground moved and the offsets held here describe a file that no longer exists.
//! Cheap to detect, and silently wrong if it is not.

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
