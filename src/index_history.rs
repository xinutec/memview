//! MEMORY.md's membership over time, recovered from the transcripts.
//!
//! Read back from wherever a session read the index, since it was versioned
//! nowhere early on.

use std::collections::{BTreeMap, BTreeSet};

/// One day's membership, by the date the reading was taken.
pub type History = BTreeMap<String, BTreeSet<String>>;

/// A reading and when it was taken, so the winner for a day can be chosen after
/// every transcript has been read rather than by arrival.
#[derive(Debug, Default)]
pub struct Readings {
    /// day → (the winning reading's full timestamp, its names)
    best: BTreeMap<String, (String, BTreeSet<String>)>,
    /// How many readings were seen in total, so a day holding several is visible.
    pub seen: usize,
}

/// The index's own path, as a transcript records it.
const INDEX: &str = "memory/MEMORY.md";

/// Is this the index being read, rather than some other file?
pub fn is_the_index(path: &str) -> bool {
    path.ends_with(INDEX)
}

/// The date part of an ISO timestamp.
pub fn day_of(stamp: &str) -> Option<&str> {
    (stamp.len() >= 10).then(|| &stamp[..10])
}

/// Pull the memory names out of one rendering of the index — link targets only,
/// so a mention is not an entry.
pub fn names_in(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut at = 0;
    while let Some(open) = text[at..].find("](") {
        let start = at + open + 2;
        let Some(close) = text[start..].find(')') else {
            break;
        };
        let target = &text[start..start + close];
        at = start + close;
        // A relative `.md` sibling and nothing else: the index also links
        // headings and the odd URL, and neither is a memory.
        let Some(stem) = target.strip_suffix(".md") else {
            continue;
        };
        if !stem.is_empty()
            && stem
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            out.insert(stem.to_string());
        }
    }
    out
}

impl Readings {
    /// Fold one reading in, keeping the day's last by timestamp — not by arrival,
    /// and not the largest.
    pub fn absorb(&mut self, stamp: &str, names: BTreeSet<String>) {
        let Some(day) = day_of(stamp) else { return };
        // An empty reading is a failed read, not an empty index.
        if names.is_empty() {
            return;
        }
        self.seen += 1;
        match self.best.get(day) {
            Some((held, _)) if held.as_str() >= stamp => {}
            _ => {
                self.best
                    .insert(day.to_string(), (stamp.to_string(), names));
            }
        }
    }

    /// The history, one membership per day.
    pub fn history(self) -> History {
        self.best
            .into_iter()
            .map(|(day, (_, names))| (day, names))
            .collect()
    }
}
