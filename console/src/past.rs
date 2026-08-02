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
    /// What the conversation calls itself — `music`, `health` — or none when it
    /// never took a name. A hex prefix identifies a transcript; only this
    /// identifies the *work*.
    pub name: Option<String>,
    /// Whether something appears to be using it already. See [`in_use`].
    pub busy: bool,
}

/// How far into a transcript to look for its working directory.
///
/// Small on purpose: these files are tens of megabytes and the answer is always
/// within the opening handful of lines. If it is not there, this transcript does
/// not say where it ran and cannot be resumed safely.
const LINES_TO_FIND_CWD: usize = 16;

/// How much of the end of a transcript to read when looking for its name.
///
/// From the **end**, because a session is renamed as its job changes and the
/// current name is the one worth showing. The name lines are re-emitted every
/// turn — two hundred times in a long conversation — so the last few kilobytes
/// always carry several, and reading a whole 1.3 GB transcript to learn one word
/// is not a trade worth making.
const TAIL_BYTES: u64 = 128 * 1024;

/// Where Claude Code keeps its transcripts.
pub fn projects_root() -> PathBuf {
    if let Ok(set) = std::env::var("CLAUDE_PROJECTS_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .join("projects")
}

/// A transcript written this recently is treated as in use.
///
/// A session writes on every turn, so anything touched in the last couple of
/// minutes almost certainly still has somebody behind it. It is a floor, not a
/// test: a conversation left open and idle for an hour looks untouched, which is
/// why the process table is consulted as well.
const RECENTLY_MILLIS: u64 = 120_000;

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
    let running = arguments();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_millis() as u64)
        .unwrap_or(0);
    for conversation in &mut found {
        conversation.busy = in_use(conversation, &running, now);
    }
    found
}

/// Whether a conversation looks like somebody else's already.
///
/// **There is no first-party answer to this**, which is why it is inferred.
/// Claude Code does not hold the transcript open while it runs — `lsof` on a live
/// session's file returns nothing — writes no lock or pid file, and leaves
/// `~/.claude/daemon/roster.json` empty for sessions started with
/// `--remote-control`. All checked, none of them work.
///
/// So two signals, and either is enough:
///
/// - **A running `claude` names it**, by session id (`--session-id`, `--resume
///   <uuid>`) or by the name it currently goes by (`--resume utterance`). Matched
///   against whole arguments of processes that really are `claude` — see
///   [`words_of_claude_processes`] for why both halves of that are load-bearing.
/// - **Its transcript was written moments ago**, which catches a session whose
///   command line says nothing useful.
///
/// Both under-detect rather than over-detect, and the cost of being wrong is
/// asymmetric: a false *busy* means a conversation cannot be resumed until it
/// goes quiet, while a false *free* means two processes appending to one
/// transcript, each blind to the other's turns. So this errs toward busy.
fn in_use(conversation: &Conversation, running: &[String], now: u64) -> bool {
    if now.saturating_sub(conversation.modified) < RECENTLY_MILLIS {
        return true;
    }
    running.iter().any(|argument| {
        argument == &conversation.id
            || conversation
                .name
                .as_deref()
                .is_some_and(|name| argument == name)
    })
}

/// Every argument of every `claude` this user is running, as separate words.
///
/// Shelled out to rather than taken from a crate: the alternative is a process
/// -inspection dependency in a binary whose whole job is to be small, to answer a
/// question `ps` already answers. An empty list — no `ps`, no permission — means
/// the freshness check stands alone, which is a weaker guard rather than none.
fn arguments() -> Vec<String> {
    let Ok(user) = std::env::var("USER") else {
        return Vec::new();
    };
    let Ok(output) = std::process::Command::new("ps")
        .args(["-u", &user, "-o", "args="])
        .output()
    else {
        return Vec::new();
    };
    words_of_claude_processes(&String::from_utf8_lossy(&output.stdout))
}

/// The words of the command lines that actually *are* `claude`.
///
/// ⚠ Not "lines mentioning claude". Every shell Claude Code spawns for a command
/// sources a snapshot under `~/.claude/`, so its whole command line — the command
/// included — matches that substring. A session called `utterance` was then held
/// as in use by any command anywhere on this machine that happened to contain the
/// word: `grep utterance`, `cd utterance`, this function being tested. The name is
/// the thing conversations are chosen by, so the false match landed exactly where
/// it hurt.
///
/// So the executable is what decides: the first word's last path element must be
/// `claude`. Arguments are read only from those lines.
pub fn words_of_claude_processes(ps_output: &str) -> Vec<String> {
    ps_output
        .lines()
        .filter(|line| {
            line.split_whitespace()
                .next()
                .and_then(|command| command.rsplit('/').next())
                .is_some_and(|name| name == "claude")
        })
        .flat_map(|line| line.split_whitespace())
        .map(|word| word.to_string())
        .collect()
}

/// How much of a transcript's end to replay when picking it up.
///
/// Generous where the name-reading tail is not: this is what somebody reads to
/// remember where they were, and a conversation cut off mid-tool-call is worse
/// than one that starts a little early. Still bounded — these files reach tens of
/// megabytes, and the last few hundred kilobytes are the last few dozen turns.
const REPLAY_BYTES: u64 = 512 * 1024;

/// And how many events of it to keep, whatever that came to.
///
/// The byte cap is about the file; this is about the screen. One tool call with a
/// large result can be most of a megabyte on its own, so bytes alone are a poor
/// proxy for how much conversation was recovered.
const REPLAY_EVENTS: usize = 400;

/// The transcript file for a session id, wherever Claude Code filed it.
///
/// Searched rather than computed, for the reason the module opens with: the
/// project-directory encoding is undocumented, and a guess is wrong silently.
pub fn transcript_of(root: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten())
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(id))
}

/// What was said before the console was watching.
///
/// Read from the end and truncated to [`REPLAY_EVENTS`], so what survives is the
/// most recent conversation rather than the oldest. A chunk taken from a byte
/// offset starts mid-line, so the first line is dropped unread.
pub fn replay(path: &Path) -> Vec<crate::protocol::Event> {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(meta) = std::fs::metadata(path) else {
        return Vec::new();
    };
    let Ok(mut file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let from = meta.len().saturating_sub(REPLAY_BYTES);
    if file.seek(SeekFrom::Start(from)).is_err() {
        return Vec::new();
    }
    let mut tail = Vec::new();
    if file.take(REPLAY_BYTES).read_to_end(&mut tail).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&tail);
    let mut lines = text.lines();
    // The first line of a mid-file chunk is a fragment. Dropped unconditionally
    // rather than tried and discarded on failure: a fragment that happens to
    // parse is worse than one that does not.
    if from > 0 {
        lines.next();
    }
    let mut events: Vec<crate::protocol::Event> =
        lines.flat_map(crate::protocol::read_recorded).collect();
    if events.len() > REPLAY_EVENTS {
        events = events.split_off(events.len() - REPLAY_EVENTS);
    }
    events
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
        name: name_of(path, meta.len()),
        // Filled in by `conversations`, which reads the process table once for
        // the whole list rather than once per file.
        busy: false,
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

/// What the transcript calls itself, if anything.
///
/// Two line types carry it: `custom-title` (what it was deliberately called) and
/// `agent-name`. The first wins where both exist, because one is a decision and
/// the other is a default.
///
/// Read from the tail — see [`TAIL_BYTES`]. A chunk taken from an arbitrary byte offset starts
/// mid-line and possibly mid-character, so the first line is expected to be
/// rubbish and unparseable lines are skipped rather than treated as the end of
/// the file.
fn name_of(path: &Path, len: u64) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};

    let mut file = std::fs::File::open(path).ok()?;
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .ok()?;
    let mut tail = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut tail).ok()?;

    let mut title = None;
    let mut agent = None;
    for line in String::from_utf8_lossy(&tail).lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if let Some(found) = value.get("customTitle").and_then(|v| v.as_str()) {
            title = Some(found.to_string());
        }
        if let Some(found) = value.get("agentName").and_then(|v| v.as_str()) {
            agent = Some(found.to_string());
        }
    }
    title.or(agent).filter(|name| !name.trim().is_empty())
}
