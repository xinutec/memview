//! Where memview keeps its own files, as opposed to where Claude Code keeps its.
//!
//! ⚠ **`~/.claude` is Anthropic's namespace and we were writing into it.** Nine
//! artefacts of ours sat directly beside Claude Code's own state — mined
//! rollups, recovered dates, a study's arms — indistinguishable in `ls` from
//! files the CLI owns and manages. Each was put there because that is where the
//! transcripts are, which is a reason for the input's location and not the
//! output's (memview#1240).
//!
//! Everything of ours now lives under one directory, so it can be found,
//! backed up, excluded or moved as one thing.
//!
//! ⚠ **This is the only place that names it.** Nine call sites across three
//! crates each built the path themselves, so a move meant nine edits and any
//! one of them could be missed silently — a tool would simply find no artefact
//! and report an empty corpus.

use std::path::PathBuf;

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

/// One of memview's files, by name.
pub fn file(name: &str) -> PathBuf {
    dir().join(name)
}

/// Claude Code's own directory — its transcripts and corpus, which we READ and
/// never write.
///
/// ⚠ Kept distinct from [`dir`] on purpose: the whole point of the split is
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
