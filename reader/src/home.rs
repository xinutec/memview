//! Where memview keeps its own files, as opposed to where Claude Code keeps its.
//!
//! `~/.claude` is Anthropic's namespace, so everything of ours lives under one
//! directory of its own, named only here.

use std::path::{Path, PathBuf};

/// The directory holding memview's own files.
///
/// `MEMVIEW_DIR` overrides it, so a test never writes to the live one and a
/// second machine can put it elsewhere.
pub fn dir() -> PathBuf {
    if let Ok(set) = std::env::var("MEMVIEW_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .join("memview")
}

/// One of memview's RECORDS, by name — a file nothing can rebuild, tracked in
/// git.
///
/// Frozen for a reason, not merely unrebuilt: re-running a pre-registered
/// judgement would be a different experiment, not a refresh.
pub fn file(name: &str) -> PathBuf {
    dir().join(name)
}

/// One of memview's CACHES, by name — rebuilt by a command, ignored by git.
///
/// A directory, not a list, so a new file lands on the right side by construction.
/// The test for this side is "does a rebuild return the same, or less?" — run it
/// and diff before moving a file here.
pub fn cache(name: &str) -> PathBuf {
    dir().join("cache").join(name)
}

/// Claude Code's own directory — its transcripts and corpus, which we READ and
/// never write.
///
/// Kept distinct from [`dir`] on purpose: the whole point of the split is
/// that a path into Anthropic's tree and a path into ours are different kinds
/// of thing, and a single `root` variable serving both is how they merged.
pub fn claude_dir() -> PathBuf {
    if let Ok(set) = std::env::var("CLAUDE_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".claude")
}

/// Where Claude Code writes session transcripts.
pub fn projects_dir() -> PathBuf {
    if let Ok(set) = std::env::var("PROJECTS_DIR") {
        return PathBuf::from(set);
    }
    claude_dir().join("projects")
}

/// The checkout root the fleet's sessions run in. `CODE_ROOT` overrides it.
pub fn code_root() -> PathBuf {
    if let Ok(set) = std::env::var("CODE_ROOT") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Code")
}

/// What Claude Code names a project's directory: its path, every character
/// that is not a letter, digit or `-` made a `-`.
pub fn project_key(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Claude Code's directory for the [`code_root`] project.
pub fn code_project_dir() -> PathBuf {
    claude_dir()
        .join("projects")
        .join(project_key(&code_root()))
}

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

/// The memory corpus. `MEMORY_DIR` overrides it.
pub fn memory_dir() -> PathBuf {
    if let Ok(set) = std::env::var("MEMORY_DIR") {
        return PathBuf::from(set);
    }
    code_project_dir().join("memory")
}
