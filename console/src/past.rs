//! Conversations that have already happened, and could be picked up again.
//!
//! The console starts processes; it cannot attach to one. A `claude` running in a
//! terminal has its stdin held by the terminal, and one started with
//! `--remote-control` talks to Anthropic over HTTPS with no local endpoint at all
//! — measured, not assumed: no listening socket and no named socket anywhere
//! under `~/.claude`. So *reaching an existing conversation* means resuming its
//! transcript in a process of our own, and this is the list of what there is to
//! resume.
//!
//! ## Read from the transcripts, not from a name we compute
//!
//! Claude Code files transcripts under `~/.claude/projects/<slug>/<id>.jsonl`,
//! where the slug is the working directory with its separators flattened. That
//! encoding is undocumented, so this does not reproduce it: it walks the
//! directories and reads each transcript's own record of its `cwd` instead. A
//! guessed encoding is wrong silently and only for the paths nobody tested —
//! a directory with a dot in it, say — and the failure looks like "there are no
//! past sessions here" rather than like a bug.
//!
//! ⚠ The `cwd` is **not** on the first line. It arrives on a `system` line a few
//! lines in, so a reader that gives up after one line finds nothing, always.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One conversation on disk.
#[derive(Debug, Clone, Serialize)]
pub struct Conversation {
    /// The session id, which is also the file's name and what `--resume` takes.
    pub id: String,
    /// Where it was running. Resuming has to happen in the same place.
    pub dir: String,
    /// When it was last written to, in milliseconds since the epoch — the only
    /// available proxy for "is anybody still using this".
    pub modified: u64,
    /// How much was said. A rough weight, and the cheap one: counting turns means
    /// reading the whole file, and these reach tens of megabytes.
    pub bytes: u64,
}

/// How far into a transcript to look for its working directory.
///
/// Small on purpose: these files are tens of megabytes and the answer is always
/// within the opening handful of lines. If it is not there, this transcript does
/// not say where it ran and cannot be resumed safely.
const LINES_TO_FIND_CWD: usize = 16;

/// Where Claude Code keeps its transcripts.
pub fn projects_root() -> PathBuf {
    if let Ok(set) = std::env::var("CLAUDE_PROJECTS_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .join("projects")
}

/// Every conversation under `root`, newest first.
pub fn conversations(root: &Path) -> Vec<Conversation> {
    let mut found: Vec<Conversation> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten())
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| read(&entry.path()))
        .collect();
    // Newest first: the one worth picking up again is almost always the last
    // one that was open.
    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.modified));
    found
}

fn read(path: &Path) -> Option<Conversation> {
    if path.extension()? != "jsonl" {
        return None;
    }
    let id = path.file_stem()?.to_str()?.to_string();
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    Some(Conversation {
        id,
        dir: cwd_of(path)?,
        modified,
        bytes: meta.len(),
    })
}

/// The working directory a transcript records for itself.
fn cwd_of(path: &Path) -> Option<String> {
    use std::io::BufRead;

    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file)
        .lines()
        .take(LINES_TO_FIND_CWD)
    {
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if let Some(cwd) = value.get("cwd").and_then(|cwd| cwd.as_str()) {
            return Some(cwd.to_string());
        }
    }
    None
}
