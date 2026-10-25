//! Conversations that have already happened, and could be picked up again.
//!
//! The console cannot attach to a running `claude`; it resumes a transcript in a
//! process of its own. Transcripts are found by walking `~/.claude/projects/` and
//! reading each one's own `cwd`, since the folder names are undocumented.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// One conversation on disk.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Conversation {
    /// The session id, which is also the file's name and what `--resume` takes.
    pub id: String,
    /// Where it was running. Resuming has to happen in the same place.
    pub dir: String,
    /// When anything last happened in it, in epoch milliseconds — [`last_moved`].
    pub modified: u64,
    /// How much was said, roughly.
    pub bytes: u64,
    /// What the conversation calls itself — `music`, `health` — if anything.
    pub name: Option<String>,
    /// How full the context was at the last recorded request, in tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub context: Option<u64>,
    /// Whether something appears to be using it already. See [`in_use`].
    pub busy: bool,
}

/// How much of a transcript to read for its working directory: the opening metadata
/// before it can run to hundreds of KB.
const BYTES_TO_FIND_CWD: u64 = 4 * 1024 * 1024;

/// How much of a transcript's end to read for its name, which is re-emitted every turn.
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

/// Temporary directories, under all three of their names.
const DISPOSABLE: [&str; 3] = ["/tmp/", "/private/tmp/", "/var/folders/"];

/// Whether this is a conversation the console's own tests made — hidden from the
/// list only.
fn disposable(dir: &str) -> bool {
    DISPOSABLE.iter().any(|temp| dir.starts_with(temp))
}

/// Every id with a transcript under `root`, unfiltered — for [`crate::images::tidy`].
pub fn transcript_ids(root: &Path) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten())
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        // A same-named directory sits beside each transcript.
        .filter(|path| reader::transcript::is_transcript(path))
        .filter_map(|path| {
            path.file_stem()
                .and_then(|stem| stem.to_str())
                .map(String::from)
        })
        .collect()
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

    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.modified));
    let running = arguments();
    for conversation in &mut found {
        conversation.busy = in_use(conversation, &running);
    }
    found
}

/// Whether a running `claude` names this conversation, by id or name — the only
/// signal: Claude Code takes no lock.
pub fn in_use(conversation: &Conversation, running: &Running) -> bool {
    // Could not ask: busy is the recoverable guess.
    let Running::Asked(running) = running else {
        return true;
    };
    running.iter().any(|argument| {
        argument == &conversation.id
            || conversation
                .name
                .as_deref()
                .is_some_and(|name| argument == name)
    })
}

/// What the process table said, or that it could not be asked.
pub enum Running {
    /// `ps` answered. Empty means nothing is running, which is a real answer.
    Asked(Vec<String>),
    /// The question could not be put at all.
    Unasked,
}

/// Every argument of every `claude` this user runs, as words.
fn arguments() -> Running {
    let Ok(user) = std::env::var("USER") else {
        // Loud: everything is now held busy.
        tracing::warn!("cannot read USER — holding every conversation busy");
        return Running::Unasked;
    };
    let output = match std::process::Command::new("ps")
        .args(["-u", &user, "-o", "args="])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            tracing::warn!("cannot run ps ({e}) — holding every conversation busy");
            return Running::Unasked;
        }
    };
    Running::Asked(words_of_claude_processes(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// The words of command lines whose program is `claude`, not ones that mention it.
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
const REPLAY_BYTES: u64 = 512 * 1024;

/// And how many events of it to keep.
const REPLAY_EVENTS: usize = 400;

/// When this session last did anything, from its transcript.
pub fn touched(root: &Path, id: &str) -> Option<u64> {
    Some(about(root, id)?.touched)
}

/// What a live session's transcript says about it, in one read.
#[derive(Debug)]
pub struct About {
    pub name: Option<String>,
    /// See [`Conversation::modified`].
    pub touched: u64,
    pub bytes: u64,
}

pub fn about(root: &Path, id: &str) -> Option<About> {
    let path = transcript_of(root, id)?;
    let meta = std::fs::metadata(&path).ok()?;
    let tail = tail_of(&path, meta.len());
    let touched = last_moved(&tail, &meta);
    Some(About {
        name: tail.name,
        touched,
        bytes: meta.len(),
    })
}

/// When this conversation last did anything: its last line of conversation, not
/// the file's date, which a resume moves.
fn last_moved(tail: &Tail, meta: &std::fs::Metadata) -> u64 {
    if let Some(spoke) = tail.spoke {
        return spoke.max(0) as u64;
    }
    meta.modified()
        .ok()
        .and_then(|when| when.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |since| since.as_millis() as u64)
}

pub fn named(root: &Path, id: &str) -> Option<String> {
    about(root, id)?.name
}

/// What a conversation is made of, for [`crate::gist`].
#[derive(Debug, Default)]
pub struct Material {
    /// The first real instruction in the file, when one can be found inside
    /// [`BYTES_TO_FIND_CWD`].
    pub opening: Option<String>,
    /// The last few things said, newest last, labelled by speaker.
    pub recent: Vec<String>,
}

/// How much of one line of conversation to keep.
const LINE: usize = 400;

/// Read a conversation down to its prompts and replies, from both ends: what it was
/// set up to do, and what it has become.
pub fn material(path: &Path, keep: usize) -> Material {
    use std::io::{BufRead, Read};

    let mut found = Material::default();
    if let Ok(file) = std::fs::File::open(path) {
        for line in std::io::BufReader::new(file.take(BYTES_TO_FIND_CWD)).lines() {
            let Ok(line) = line else { break };
            // The first `Prompt` is the first thing a person said.
            if let Some(text) =
                crate::protocol::read_recorded(&line)
                    .into_iter()
                    .find_map(|event| match event {
                        crate::protocol::Event::Prompt { text } => Some(text),
                        _ => None,
                    })
            {
                found.opening = Some(cut(&text));
                break;
            }
        }
    }
    for timed in page(path, None).events {
        match timed.event {
            crate::protocol::Event::Prompt { text } => {
                found.recent.push(format!("them: {}", cut(&text)));
            }
            crate::protocol::Event::Text { text } if !text.trim().is_empty() => {
                found.recent.push(format!("agent: {}", cut(&text)));
            }
            _ => {}
        }
    }
    if found.recent.len() > keep {
        found.recent.drain(..found.recent.len() - keep);
    }
    found
}

/// One line of it, at a length that leaves room for the rest.
fn cut(text: &str) -> String {
    let text = text.trim();
    match text.char_indices().nth(LINE) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// Exchanges since the last compaction, counted from the file forward from where
/// the last count stopped.
#[derive(Debug, Clone, Copy, Default, Serialize, serde::Deserialize)]
pub struct Counted {
    /// Exchanges since the last compaction.
    pub interactions: u32,
    /// How far into the file that accounts for — a line boundary.
    pub through: u64,
    /// When the newest answer read so far was written, in epoch milliseconds.
    #[serde(default)]
    pub answered_at: i64,
}

/// What the bytes appended since last time hold.
#[derive(Debug, Clone, Default)]
pub struct Appended {
    pub counted: Counted,
    /// Background tasks reported finished — known only from the file.
    pub finished: Vec<crate::protocol::Named>,
    /// Whether a compaction was filed among them.
    pub compacted: bool,
    /// The newest fullness recorded after any compaction.
    pub context: Option<u64>,
    /// The newest answer's model and effort: a `/model` or `/effort` switch is recorded
    /// nowhere else.
    pub answered: Option<crate::protocol::Answered>,
}

/// A place in a transcript to come back to: something said or sent, or a compaction.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Landmark {
    /// The cursor to ask `/api/sessions/{id}/earlier` for: the end of its line, since
    /// pages read backwards.
    pub at: u64,
    /// When the file says it happened, for grouping by day.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub when: Option<i64>,
    pub kind: Mark,
    /// Enough of it to recognise, cut to [`SIGN`].
    pub text: String,
}

/// Which kind of landmark, in the client's vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Mark {
    /// Something the reader said.
    Prompt,
    /// A slash command they ran.
    Command,
    /// Pictures they sent, by what was said with them.
    Shown,
    /// Where the conversation was cut and started again.
    Compacted,
}

/// How much of a landmark to carry: shorter than [`LINE`], a strip of things to tap.
const SIGN: usize = 120;

/// Every landmark in a transcript, oldest first. Parses the whole file — seconds;
/// call it off the executor. A byte prescan would miss most landmarks.
pub fn landmarks(path: &Path) -> Vec<Landmark> {
    landmarks_from(path, 0).found
}

/// What one walk read, and the byte the next one should start at.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    pub found: Vec<Landmark>,
    /// One past the last complete line read.
    pub through: u64,
}

/// The same walk from a previous walk's [`Walk::through`]. A half-written last line
/// is left for next time.
pub fn landmarks_from(path: &Path, from: u64) -> Walk {
    use std::io::BufRead;
    use std::io::Seek;

    let Ok(mut file) = std::fs::File::open(path) else {
        return Walk::default();
    };
    if from > 0 && file.seek(std::io::SeekFrom::Start(from)).is_err() {
        return Walk::default();
    }
    let mut reader = std::io::BufReader::with_capacity(1 << 20, file);
    let mut line = Vec::new();
    let mut at = from;
    let mut found = Vec::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_until(b'\n', &mut line) else {
            break;
        };
        if read == 0 || !line.ends_with(b"\n") {
            break;
        }
        let past_it = at + read as u64;
        for stamped in timed(&line) {
            let (kind, text) = match stamped.event {
                crate::protocol::Event::Prompt { text } => (Mark::Prompt, text),
                crate::protocol::Event::Command { text } => (Mark::Command, text),
                crate::protocol::Event::Shown { names, text } => {
                    (Mark::Shown, shown_sign(&names, text))
                }
                crate::protocol::Event::Compacted => (Mark::Compacted, String::new()),
                _ => continue,
            };
            found.push(Landmark {
                at: past_it,
                when: stamped.at,
                kind,
                text: sign(&text),
            });
        }
        at = past_it;
    }
    Walk { found, through: at }
}

/// What a picture message is found by: its words, or how many pictures when it
/// has none and there are several. One wordless picture is left for the client
/// to name.
fn shown_sign(names: &[String], text: String) -> String {
    match (text.is_empty(), names.len()) {
        (false, _) | (true, 0 | 1) => text,
        (true, many) => format!("{many} pictures"),
    }
}

/// One line of a landmark, cut to [`SIGN`].
fn sign(text: &str) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(SIGN) {
        Some((end, _)) => format!("{}…", &flat[..end]),
        None => flat,
    }
}

/// How far back to look for the compaction boundary, and how much to read first.
/// The last compaction sits within the final megabytes.
const COMPACTION_WINDOW: u64 = 8 * 1024 * 1024;
const COMPACTION_LIMIT: u64 = 64 * 1024 * 1024;

/// Where a seed can start counting with the same result: the last compaction, which
/// resets the count. 0 when none is within [`COMPACTION_LIMIT`].
pub fn seed_from(path: &Path) -> u64 {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(meta) = std::fs::metadata(path) else {
        return 0;
    };
    let end = meta.len();
    let Ok(mut file) = std::fs::File::open(path) else {
        return 0;
    };

    let mut span = COMPACTION_WINDOW;
    loop {
        let start = end.saturating_sub(span);
        if file.seek(SeekFrom::Start(start)).is_err() {
            return 0;
        }
        let mut buf = Vec::new();
        // `by_ref`: the span may widen.
        if file
            .by_ref()
            .take(end - start)
            .read_to_end(&mut buf)
            .is_err()
        {
            return 0;
        }

        let mut at = start;
        let mut found = None;
        for (index, line) in buf.split_inclusive(|byte| *byte == b'\n').enumerate() {
            // A mid-file chunk's first line is a fragment.
            let whole = start == 0 || index > 0;
            if whole
                && crate::protocol::read_recorded(&String::from_utf8_lossy(line))
                    .iter()
                    .any(|event| matches!(event, crate::protocol::Event::Compacted))
            {
                found = Some(at);
            }
            at += line.len() as u64;
        }
        if let Some(offset) = found {
            return offset;
        }
        if start == 0 || span >= COMPACTION_LIMIT {
            return 0;
        }
        span = span.saturating_mul(2);
    }
}

/// Count what was appended since `so_far`, stopping before a partial last line. A
/// file that shrank was replaced: start again.
pub fn counted(path: &Path, so_far: Counted) -> Appended {
    use std::io::{BufRead, Seek, SeekFrom};

    let mut finished = Vec::new();
    let mut compacted = false;
    let mut context = None;
    let mut answered = None;
    let Ok(mut file) = std::fs::File::open(path) else {
        return Appended {
            counted: so_far,
            finished,
            compacted: false,
            context: None,
            answered,
        };
    };
    let len = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let mut found = if so_far.through > len {
        Counted::default()
    } else {
        so_far
    };
    if file.seek(SeekFrom::Start(found.through)).is_err() {
        return Appended {
            counted: so_far,
            finished,
            compacted: false,
            context: None,
            answered,
        };
    }
    let mut reader = std::io::BufReader::new(file);
    let mut line = Vec::new();
    loop {
        line.clear();
        let Ok(read) = reader.read_until(b'\n', &mut line) else {
            break;
        };
        if read == 0 || !line.ends_with(b"\n") {
            break;
        }
        found.through += read as u64;
        let text = String::from_utf8_lossy(&line);
        // Not the last line's: a compaction appends older history after the newest.
        if let Some(answer) = crate::protocol::answered(&text)
            && answer.at >= found.answered_at
        {
            found.answered_at = answer.at;
            answered = Some(answer);
        }
        for event in crate::protocol::read_recorded(&text) {
            match event {
                crate::protocol::Event::Compacted => {
                    found.interactions = 0;
                    compacted = true;
                    // Measured before the boundary: stale.
                    context = None;
                }
                // For the compaction case.
                crate::protocol::Event::Context { tokens } => context = Some(tokens),
                crate::protocol::Event::Prompt { .. } => found.interactions += 1,
                // See [`Appended::finished`].
                ref background @ crate::protocol::Event::Background { .. } => {
                    if let crate::protocol::Running::Ended(named) =
                        crate::protocol::running(background)
                    {
                        finished.push(named);
                    }
                }
                _ => {}
            }
        }
    }
    Appended {
        counted: found,
        finished,
        compacted,
        context,
        answered,
    }
}

/// The transcript file for a session id, found by search.
pub fn transcript_of(root: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten())
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        // The extension: the stem alone matches the directory beside it.
        .filter(|path| reader::transcript::is_transcript(path))
        .find(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(id))
}

/// The directory one conversation was working in, by id, without listing anything.
pub fn dir_of(root: &Path, id: &str) -> Option<String> {
    cwd_of(&transcript_of(root, id)?)
}

/// One page of a transcript, and where in the file it started. `from` is the
/// cursor the reader hands back for the page before; 0 means nothing older.
#[derive(Debug)]
pub struct Page {
    pub events: Vec<crate::protocol::Timed>,
    pub from: u64,
}

/// Every event one transcript line carries, each with the line's time.
fn timed(line: &[u8]) -> Vec<crate::protocol::Timed> {
    let text = String::from_utf8_lossy(line);
    let at = crate::protocol::recorded_at(&text);
    crate::protocol::read_recorded(&text)
        .into_iter()
        .map(|event| crate::protocol::Timed { at, event })
        .collect()
}

/// One page of what was said, before cursor `before` — or the newest. The span
/// doubles until it holds a page.
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
        // `by_ref`: the span may widen.
        if file
            .by_ref()
            .take(end - start)
            .read_to_end(&mut buf)
            .is_err()
        {
            return empty;
        }

        // Line starts as byte offsets: decoding could shift them.
        let mut lines: Vec<(u64, &[u8])> = Vec::new();
        let mut at = start;
        for line in buf.split_inclusive(|byte| *byte == b'\n') {
            lines.push((at, line));
            at += line.len() as u64;
        }
        // A mid-file chunk's first line is a fragment.
        if start > 0 && !lines.is_empty() {
            lines.remove(0);
        }

        // Whole lines back from the newest, so the cursor is a line boundary.
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

        // More file behind a short page: widen.
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

/// Where the last whole line of a transcript ends.
pub fn written(path: &Path) -> u64 {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| bytes.iter().rposition(|byte| *byte == b'\n'))
        .map_or(0, |at| at as u64 + 1)
}

/// The events of every whole line after cursor `from`, and the cursor past them.
pub fn since(path: &Path, from: u64) -> (Vec<crate::protocol::Timed>, u64) {
    use std::io::{Read, Seek, SeekFrom};

    let mut buf = Vec::new();
    let read = std::fs::File::open(path).and_then(|mut file| {
        file.seek(SeekFrom::Start(from))?;
        file.read_to_end(&mut buf)
    });
    if read.is_err() {
        return (Vec::new(), from);
    }
    let whole = buf
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(0, |at| at + 1);
    let events = buf[..whole]
        .split_inclusive(|byte| *byte == b'\n')
        .flat_map(timed)
        .collect();
    (events, from + whole as u64)
}

fn read(path: &Path) -> Option<Conversation> {
    if !reader::transcript::is_transcript(path) {
        return None;
    }
    let id = path.file_stem()?.to_str()?.to_string();
    let meta = std::fs::metadata(path).ok()?;
    let tail = tail_of(path, meta.len());
    Some(Conversation {
        id,
        dir: cwd_of(path)?,
        modified: last_moved(&tail, &meta),
        bytes: meta.len(),
        name: tail.name,
        context: tail.context,
        // Filled by `conversations`.
        busy: false,
    })
}

/// The working directory a transcript records, within [`BYTES_TO_FIND_CWD`].
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

/// What the end of a transcript says, last one winning, in one pass.
#[derive(Debug, Default)]
struct Tail {
    /// See [`Conversation::name`].
    name: Option<String>,
    /// See [`Conversation::context`].
    context: Option<u64>,
    /// See [`Conversation::modified`].
    spoke: Option<i64>,
}

/// The first thing this session was asked, from the head of its transcript —
/// `None` for a continued session, whose file opens with a summary instead.
pub fn opening(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader, Read};

    let file = std::fs::File::open(path).ok()?;
    // A continued transcript opens with its whole summary.
    let head = BufReader::new(file.take(OPENING_BYTES));
    for line in head.lines().map_while(Result::ok) {
        for event in crate::protocol::read_recorded(&line) {
            if let crate::protocol::Event::Prompt { text } = event {
                if CONTINUED
                    .iter()
                    .any(|opener| text.trim_start().starts_with(opener))
                {
                    return None;
                }
                return Some(text);
            }
        }
    }
    None
}

/// How a continued transcript opens, in the harness's words.
const CONTINUED: [&str; 2] = [
    "This session is being continued from a previous conversation",
    "Caveat: The messages below were generated by the user while running local commands",
];

/// How much of a transcript's head to read looking for its first prompt.
const OPENING_BYTES: u64 = 1024 * 1024;

fn tail_of(path: &Path, len: u64) -> Tail {
    use std::io::{Read, Seek, SeekFrom};

    let mut found = Tail::default();
    let Ok(mut file) = std::fs::File::open(path) else {
        return found;
    };
    if file
        .seek(SeekFrom::Start(len.saturating_sub(TAIL_BYTES)))
        .is_err()
    {
        return found;
    }
    let mut tail = Vec::new();
    if file.take(TAIL_BYTES).read_to_end(&mut tail).is_err() {
        return found;
    }

    // Precedence comes from the shared order.
    let mut names: std::collections::BTreeMap<&'static str, String> =
        std::collections::BTreeMap::new();
    for line in String::from_utf8_lossy(&tail).lines() {
        let events = crate::protocol::read_recorded(line);
        // The last line that is conversation is the last thing that happened.
        if !events.is_empty()
            && let Some(at) = crate::protocol::recorded_at(line)
        {
            found.spoke = Some(at);
        }
        for event in events {
            match event {
                crate::protocol::Event::Context { tokens } => found.context = Some(tokens),
                // Stale above the boundary; a later request sets it again.
                crate::protocol::Event::Compacted => found.context = None,
                _ => {}
            }
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // Shared field names; the last of each kind wins.
        for line in reader::transcript::AS_CONVERSATION {
            if let Some(name) = value.get(line.field).and_then(|v| v.as_str()) {
                names.insert(line.field, name.to_string());
            }
        }
    }
    // A chosen title wins — [`reader::transcript::AS_CONVERSATION`].
    found.name = reader::transcript::AS_CONVERSATION
        .iter()
        .find_map(|line| names.get(line.field))
        .filter(|name| !name.trim().is_empty())
        .cloned();
    found
}
