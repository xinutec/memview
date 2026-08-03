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
//! ⚠ The `cwd` is **not** on the first line. Every line of the conversation
//! carries it and every line of the opening metadata omits it, so the search is
//! "read until one appears", bounded by bytes — see [`BYTES_TO_FIND_CWD`] for why
//! bounding it by a number of lines cannot be right at any value.

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

/// How much of a transcript to read while looking for its working directory.
///
/// ⚠ **A budget in bytes, and it took two wrong answers to get here.** This was a
/// count of lines — 16, then 64 — and a count of lines cannot be right at any
/// value, because the number of lines before the first conversation line is data
/// rather than format. A transcript opens with metadata that carries no `cwd`
/// (`mode`, `permission-mode`, `queue-operation`, `file-history-snapshot`) and how
/// many of those there are depends on how many files the session had open. Twelve
/// of the thirteen transcripts on this machine reach `cwd` inside 1 KB; one opens
/// with twelve `file-history-snapshot` lines and reaches it at **456 KB**. At 16
/// that conversation was missing from the list entirely, and 64 would have hidden
/// the next session that happened to hold thirty files instead of twelve.
///
/// Bytes are what the bound is actually for: these files reach 1.4 GB, and the
/// promise worth making is "never read much of one", which is a statement about
/// cost. A line count dressed that up as a statement about the format, and the
/// format does not support it. Set roughly ten times the largest opening measured
/// — headroom is cheap here, and being short is invisible.
const BYTES_TO_FIND_CWD: u64 = 4 * 1024 * 1024;

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

/// The directories a conversation is not worth listing from.
///
/// Three prefixes rather than one because they are the same place: `/tmp` is a
/// symlink to `/private/tmp` on macOS, so which of the two a transcript records
/// depends on how its process was started, and `$TMPDIR` is a third path again
/// under `/var/folders`. Checking one of them is the version that looks right and
/// silently misses.
const DISPOSABLE: [&str; 3] = ["/tmp/", "/private/tmp/", "/var/folders/"];

/// Whether this is a conversation the console made while testing itself.
///
/// The spawner takes a working directory, and pointing it at a scratchpad is how
/// its own behaviour gets tested. Claude Code files transcripts per working
/// directory, so every probe became a project directory beside the real ones —
/// nine of them from one afternoon, one to five turns each, and nothing in the
/// list distinguished them from a conversation worth picking up.
///
/// Judged on the working directory a transcript records, not on the name of the
/// folder holding it, for the reason this module opens with: the folder-name
/// encoding is undocumented and a guess at it is wrong silently.
///
/// ⚠ This hides a conversation that genuinely ran from a temporary directory. It
/// is a display filter and nothing more — [`transcript_of`] still finds any
/// session by id, so such a conversation stays resumable by name; it just stops
/// competing for room on a phone screen with the repositories this exists to get
/// back to.
fn disposable(dir: &str) -> bool {
    DISPOSABLE.iter().any(|temp| dir.starts_with(temp))
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
        .filter(|conversation| !disposable(&conversation.dir))
        .collect();
    // Newest first: the one worth picking up again is almost always the last
    // one that was open.
    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.modified));
    let running = arguments();
    for conversation in &mut found {
        conversation.busy = in_use(conversation, &running);
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
/// **One signal: a running `claude` names it** — by session id (`--session-id`,
/// `--resume <uuid>`) or by the name it currently goes by (`--resume utterance`).
/// Matched against whole arguments of processes that really are `claude`; see
/// [`words_of_claude_processes`] for why both halves of that are load-bearing.
///
/// ⚠ **There used to be a second signal, and it was the wrong kind.** A
/// transcript written in the last two minutes was also treated as in use, to
/// catch a session whose command line says nothing useful. It caught the wrong
/// thing far more often: every conversation this console had *just* stopped
/// running looked busy for two minutes afterwards, so restarting the runner meant
/// waiting before the session it had killed could be picked up again — the exact
/// moment somebody wants it back. Removed on Pippijn's instruction, 2026-08-03,
/// with the reason that matters: nothing on this machine runs `claude` except
/// this console, and what this console runs it kills on the way out
/// (`kill_on_drop`), so the process table is accurate the instant it matters.
///
/// The risk that remains is a session started outside the console whose command
/// line names neither its id nor its name. That would read as free and could be
/// resumed underneath, giving two processes one transcript. The freshness rule
/// did guard that case — it just charged everybody two minutes for it.
fn in_use(conversation: &Conversation, running: &[String]) -> bool {
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

/// What a live session calls itself, read from the transcript it is writing.
///
/// The name is not on the wire — the CLI writes `customTitle`/`agentName` to
/// its transcript and announces neither on stdout — so a running session can
/// only be named by reading the file it is filling in. Read per request rather
/// than cached because a session is renamed as its job changes, and a stale
/// name on screen is worse than none: it says the wrong thing confidently.
///
/// Cheap despite the file being enormous: [`name_of`] reads the last
/// [`TAIL_BYTES`] and no more.
pub fn named(root: &Path, id: &str) -> Option<String> {
    facts(root, id).name
}

/// What a live session says about itself that the stream never mentions.
///
/// Both of these are written to the transcript and announced nowhere else, and
/// both change while a session runs — it is renamed as its job changes, and the
/// permission mode is whatever it was last set to. So both are read per request
/// from the file, and read **together**: one 128 KiB tail rather than two.
#[derive(Debug, Default, Clone)]
pub struct Facts {
    pub name: Option<String>,
    /// `default`, `auto`, `acceptEdits`, `plan`, `bypassPermissions`,
    /// `dontAsk` — the CLI's own vocabulary, read off the 2.1.220 binary's
    /// declared enum rather than guessed. `None` until the session records one.
    pub mode: Option<String>,
}

/// See [`Facts`].
pub fn facts(root: &Path, id: &str) -> Facts {
    let Some(path) = transcript_of(root, id) else {
        return Facts::default();
    };
    let Ok(len) = std::fs::metadata(&path).map(|meta| meta.len()) else {
        return Facts::default();
    };
    facts_of(&path, len)
}

/// How many times someone has spoken to this session since it was last compacted.
///
/// **Exchanges, not messages.** The result line's `num_turns` counts the
/// assistant messages one exchange took — measured: two exchanges reported 5 and
/// 8, and the transcript holds exactly 5 and 8 assistant replies for them. That
/// is a fine number for a bill and a poor one for a person, who wants to know how
/// many times they have asked for something.
///
/// **Counted from the file rather than kept as a running total**, because a
/// running total only ever counts from whenever this console picked the session
/// up: a resumed conversation started at zero, and every in-place upgrade
/// restarted it. The file is the one place that knows the whole of it.
///
/// The count resets at each compaction, since that is the point at which the
/// session stops remembering what came before — a number spanning a boundary
/// would describe a conversation the session itself cannot recall.
///
/// A whole-file pass, so this belongs at a seed or the end of a turn, not on
/// every request.
pub fn interactions(path: &Path) -> u32 {
    use std::io::BufRead;
    let Ok(file) = std::fs::File::open(path) else {
        return 0;
    };
    let mut since = 0;
    for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
        for event in crate::protocol::read_recorded(&line) {
            match event {
                crate::protocol::Event::Compacted => since = 0,
                crate::protocol::Event::Prompt { .. } => since += 1,
                _ => {}
            }
        }
    }
    since
}

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

/// One page of a transcript, and where in the file it started.
///
/// `from` is the cursor: the byte offset of the first line this page contains.
/// It is what the reader hands back to ask for the page before it, and `from ==
/// 0` is the only honest way to say there is nothing older.
#[derive(Debug)]
pub struct Page {
    pub events: Vec<crate::protocol::Timed>,
    pub from: u64,
}

/// Every event one transcript line carries, each wearing that line's time.
///
/// The stamp is per *line*, and a line can hold several events — an assistant
/// message with two tool calls in it. They share the time, which is what the file
/// says: it recorded when the message arrived, not when each block within it did.
fn timed(line: &[u8]) -> Vec<crate::protocol::Timed> {
    let text = String::from_utf8_lossy(line);
    let at = crate::protocol::recorded_at(&text);
    crate::protocol::read_recorded(&text)
        .into_iter()
        .map(|event| crate::protocol::Timed { at, event })
        .collect()
}

/// What was said before the console was watching, and the page before that one.
///
/// `before` is a cursor from a previous [`Page`], or `None` for the newest page.
///
/// ⚠ **A cursor, not a count.** This took a count of events the reader held and
/// worked backwards from the end of the file, which is wrong twice over. The
/// file grows, so counting from its end names a different place after every turn;
/// and a count travels through a client that holds *folded entries* — several
/// text deltas are one paragraph, a tool result belongs to its call — so the
/// number that arrived was never the number that left. Measured against a live
/// session: a reader holding 266 events asked for the page before them as 170,
/// and every one of the 96 events it got back was already on its screen. The
/// feature could not advance, and both quantities were `usize`, so nothing said
/// so.
///
/// A byte offset into an append-only file has neither problem: it survives the
/// file growing, and it cannot be confused with a length by anything that
/// carries it.
///
/// Re-read and re-parsed each time rather than kept: the alternative is holding a
/// parsed copy of a file that reaches tens of megabytes, for a session that may
/// never be scrolled at all. The span doubles until it has a page's worth, so
/// reaching further back costs more only when somebody actually goes there.
pub fn page(path: &Path, before: Option<u64>) -> Page {
    use std::io::{Read, Seek, SeekFrom};

    let empty = Page {
        events: Vec::new(),
        from: 0,
    };
    let Ok(meta) = std::fs::metadata(path) else {
        return empty;
    };
    let end = before.unwrap_or(meta.len()).min(meta.len());
    if end == 0 {
        return empty;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return empty;
    };

    let mut span = REPLAY_BYTES;
    loop {
        let start = end.saturating_sub(span);
        if file.seek(SeekFrom::Start(start)).is_err() {
            return empty;
        }
        let mut buf = Vec::new();
        // `by_ref`, because `take` consumes the reader and the span may have to
        // widen and read again.
        if file
            .by_ref()
            .take(end - start)
            .read_to_end(&mut buf)
            .is_err()
        {
            return empty;
        }

        // Line starts as absolute offsets, so a cursor can name one. Bytes
        // rather than a decoded string: lossy decoding can change byte lengths,
        // and an offset that is off by one names the middle of a line.
        let mut lines: Vec<(u64, &[u8])> = Vec::new();
        let mut at = start;
        for line in buf.split_inclusive(|byte| *byte == b'\n') {
            lines.push((at, line));
            at += line.len() as u64;
        }
        // The first line of a mid-file chunk is a fragment. Dropped
        // unconditionally rather than tried and discarded on failure: a fragment
        // that happens to parse is worse than one that does not.
        if start > 0 && !lines.is_empty() {
            lines.remove(0);
        }

        // Backwards from the newest, taking whole lines until the page is full.
        // Whole lines because the cursor has to name a line boundary — half a
        // line is not a place a reader can come back to.
        let mut taken = 0usize;
        let mut first = lines.len();
        for (index, (_, line)) in lines.iter().enumerate().rev() {
            let events = timed(line);
            if taken > 0 && taken + events.len() > REPLAY_EVENTS {
                break;
            }
            taken += events.len();
            first = index;
        }

        // Ran out of buffer before filling the page, and there is more file
        // behind it: widen and start again rather than return a short page that
        // would read as "this is all there is".
        if first == 0 && start > 0 && taken < REPLAY_EVENTS {
            span = span.saturating_mul(2);
            continue;
        }

        let events: Vec<crate::protocol::Timed> = lines[first..]
            .iter()
            .flat_map(|(_, line)| timed(line))
            .collect();
        let from = lines.get(first).map_or(0, |(offset, _)| *offset);
        return Page { events, from };
    }
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
        name: facts_of(path, meta.len()).name,
        // Filled in by `conversations`, which reads the process table once for
        // the whole list rather than once per file.
        busy: false,
    })
}

/// The working directory a transcript records for itself.
///
/// The first line that carries one answers it, and no line before then can: every
/// line of the conversation proper — `system`, `user`, `assistant`, `attachment` —
/// records the directory, and every line of the opening metadata omits it. So this
/// reads forward until one appears rather than looking in a particular place.
///
/// The reader is capped by [`BYTES_TO_FIND_CWD`] rather than the loop counting,
/// which also bounds how much a single line can cost: one `file-history-snapshot`
/// runs to tens of kilobytes and nothing in the format promises an upper bound.
/// A line cut off by the cap fails to parse and is skipped, which is the intended
/// end of the search.
fn cwd_of(path: &Path) -> Option<String> {
    use std::io::{BufRead, Read};

    let file = std::fs::File::open(path).ok()?;
    for line in std::io::BufReader::new(file.take(BYTES_TO_FIND_CWD)).lines() {
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
fn facts_of(path: &Path, len: u64) -> Facts {
    use std::io::{Read, Seek, SeekFrom};

    let mut facts = Facts::default();
    let Ok(mut file) = std::fs::File::open(path) else {
        return facts;
    };
    if file
        .seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .is_err()
    {
        return facts;
    }
    let mut tail = Vec::new();
    if file.take(TAIL_BYTES).read_to_end(&mut tail).is_err() {
        return facts;
    }

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
        // ⚠ **Keyed on the line type, not on the field.** `permissionMode` is on
        // six kinds of line in one real transcript — including `text` and
        // `assistant`, where it is a conversation that happened to mention the
        // word. A scan for the field alone reads the mode off somebody quoting
        // it. `permission-mode` lines are the CLI's own record of the setting.
        if value.get("type").and_then(|v| v.as_str()) == Some("permission-mode")
            && let Some(found) = value.get("permissionMode").and_then(|v| v.as_str())
        {
            facts.mode = Some(found.to_string());
        }
    }
    facts.name = title.or(agent).filter(|name| !name.trim().is_empty());
    facts
}
