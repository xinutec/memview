//! Which lines of `MEMORY.md` a session is actually given.
//!
//! The index is injected whole or not at all — past a size limit Claude Code delivers
//! a prefix and says so, but it never says WHAT it withheld. So a root over the line
//! does not read as broken: every session sees a complete-looking file that stops
//! early, and the memories below the cut are indistinguishable from memories nobody
//! wrote.
//!
//! This is computed from the file, not mined from a transcript. The injected
//! copy arrives in the system prompt, which no transcript entry carries. Recovering
//! the cut from the record is therefore impossible, and recomputing it is better
//! than mining would have been: it is a pure function of the file.

/// The size `MEMORY.md` is truncated at when injected, from Claude Code's own
/// warning text.
///
/// MEASURED, and it was a guess before that. Claude Code prints the size and
/// the limit in the same units — `MEMORY.md is 25.7KB (limit: 24.4KB)` — so the only
/// question was which kilobyte, and the file's own git history answers it. Every size
/// the root has ever had, against the two readings of the two values the harness has
/// been observed printing:
///
/// ```text
/// decimal KB   "24.6KB" ← 24,613 b     "25.7KB" ← 25,684 b
/// binary KiB   no size ever            no size ever
/// ```
///
/// Decimal, unambiguously: each printed value maps to a size that existed, and under
/// the binary reading neither corresponds to any size the file has ever been. **So
/// the limit is 24,400 bytes and this is the edge, not a warning line.**
///
/// The corpus header's bracket is FALSIFIED by this — it reads as though the
/// limit sits between two observed sizes. The upper one is real; the lower is not,
/// because the whole-arrival was measured under the limit and brackets nothing.
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
/// Whole lines, which over-reports the loss by at most one partial line. Whether
/// the harness cuts on a line boundary or at an exact byte is not established, and a
/// byte cut would deliver one more fragment than this reports. Erring that way is the
/// right direction — a half-delivered index line names a memory whose teaser is
/// chopped — and it is the reading that cannot quietly say "you have it all" when a
/// session does not.
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
