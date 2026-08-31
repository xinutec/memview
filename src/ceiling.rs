//! Which lines of `MEMORY.md` a session is actually given.
//!
//! The index is injected whole or not at all — past a size limit Claude Code
//! delivers a prefix and says so, but it never says WHAT it withheld. So a root
//! over the line does not read as broken: every session sees a complete-looking
//! file that stops early, and the memories below the cut are indistinguishable
//! from memories nobody wrote.
//!
//! ⚠ **This is computed from the file, not mined from a transcript.** The
//! injected copy is not persisted anywhere — it arrives in the system prompt,
//! which no transcript entry carries (checked across the corpus: the only
//! entries holding the index are `Read` results and prompts that mention it).
//! Recovering the cut from the record is therefore impossible, and recomputing
//! it is better than mining would have been: it is a pure function of the file
//! and needs no session to have happened.

/// The size `MEMORY.md` is truncated at when injected, from Claude Code's own
/// warning text.
///
/// ⚠ **Deliberately the LOW reading of an ambiguous figure**, and the same
/// number `memory-tiers` administers the trade by: 24.4 KB is either 24,400 or
/// 24,985 bytes, and guessing high costs a silent truncation while guessing low
/// costs a few hundred bytes of headroom. The observed edge is wider still and
/// has never been pinned — a root of about 25 KB arrived whole, one of 27,382
/// bytes lost its last forty entries. So this is where a warning is worth
/// raising, not where the cliff is.
pub const INDEX_CEILING: usize = 24_400;

/// What a session is given, and what it is not.
///
/// Two slices of the same string rather than a size and a list, so the halves
/// cannot drift: `kept.len()` is the delivered size and `dropped` is the text
/// itself, which [`crate::store::index_links`] will name the casualties from
/// without a second parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut<'a> {
    /// The prefix a session receives — always a whole number of lines.
    pub kept: &'a str,
    /// The tail it does not, empty when the whole file arrives.
    pub dropped: &'a str,
}

impl Cut<'_> {
    /// Whether the file arrives intact.
    pub fn is_whole(&self) -> bool {
        self.dropped.is_empty()
    }
}

/// The largest prefix of whole lines that fits under `ceiling`.
///
/// ⚠ **Whole lines, which over-reports the loss by at most one partial line.**
/// The single observation available ended on a complete entry; whether the
/// harness cuts on a line boundary or at an exact byte is not established, and
/// a byte cut would deliver one more fragment than this reports. Erring that way
/// is the right direction — a half-delivered index line names a memory whose
/// teaser is chopped, which is not a line anybody can rely on — and it is the
/// reading that cannot quietly say "you have it all" when a session does not.
pub fn cut(index: &str, ceiling: usize) -> Cut<'_> {
    if index.len() <= ceiling {
        return Cut {
            kept: index,
            dropped: "",
        };
    }
    // `split_inclusive` keeps the newline on the line it terminates, so the
    // running total is the byte offset the next line starts at — no arithmetic
    // that has to remember whether a separator was counted.
    let mut end = 0usize;
    for line in index.split_inclusive('\n') {
        let next = end + line.len();
        if next > ceiling {
            break;
        }
        end = next;
    }
    Cut {
        kept: &index[..end],
        dropped: &index[end..],
    }
}
