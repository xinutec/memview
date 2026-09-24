//! Conversations that have already happened, and could be picked up again.
//!
//! The console starts processes; it cannot attach to one — a `claude` in a
//! terminal has its stdin, and `--remote-control` talks to Anthropic with no local
//! endpoint. Reaching a conversation means resuming its transcript in a process
//! of our own.
//!
//! Transcripts live under `~/.claude/projects/<slug>/<id>.jsonl`, where the slug is
//! an undocumented flattening of the working directory. This does not reproduce
//! it: it walks the directories and reads each transcript's own `cwd`, which is
//! not on the first line — see [`BYTES_TO_FIND_CWD`].

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
    /// When anything last happened in it, in milliseconds since the epoch — from the
    /// last line of the conversation, not the file's date; see [`last_moved`].
    pub modified: u64,
    /// How much was said. A rough weight, and the cheap one.
    pub bytes: u64,
    /// What the conversation calls itself — `music`, `health` — or none. A hex prefix
    /// identifies a transcript; only this identifies the work.
    pub name: Option<String>,
    /// How full the context was at the last request recorded, in tokens. No window
    /// to divide by: that is declared on the CLI's stdout, never in the file. `None`
    /// when the tail holds no assistant message.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub context: Option<u64>,
    /// Whether something appears to be using it already. See [`in_use`].
    pub busy: bool,
}

/// How much of a transcript to read while looking for its working directory. A
/// budget in bytes, not lines: the opening metadata carries no `cwd`, and how
/// many such lines there are depends on how many files the session had open — one
/// transcript reaches it at 456 KB. Well past the largest opening seen.
const BYTES_TO_FIND_CWD: u64 = 4 * 1024 * 1024;

/// How much of the end of a transcript to read when looking for its name. From
/// the END: a session is renamed as its job changes, and the name lines are
/// re-emitted every turn.
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

/// The directories a conversation is not worth listing from. Three prefixes for
/// one place: `/tmp` is a symlink to `/private/tmp`, and `$TMPDIR` is a third
/// path under `/var/folders`.
const DISPOSABLE: [&str; 3] = ["/tmp/", "/private/tmp/", "/var/folders/"];

/// Whether this is a conversation the console made while testing itself — every
/// probe becomes a project directory beside the real ones. Judged on the recorded
/// working directory, not the folder name. A display filter only:
/// [`transcript_of`] still finds any session by id.
fn disposable(dir: &str) -> bool {
    DISPOSABLE.iter().any(|temp| dir.starts_with(temp))
}

/// Every id that has a transcript under `root`, filtered by nothing — unlike
/// [`conversations`], the display list. Asked by the housekeeping that deletes;
/// see [`crate::images::tidy`].
pub fn transcript_ids(root: &Path) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten())
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        // The extension, for the reason [`transcript_of`] gives: a directory sits beside
        // each transcript with the same name.
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
    // Newest first: the one worth picking up is almost always the last one open.
    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.modified));
    let running = arguments();
    for conversation in &mut found {
        conversation.busy = in_use(conversation, &running);
    }
    found
}

/// Whether a conversation looks like somebody else's already.
///
/// There is no first-party answer — Claude Code holds no handle and writes no
/// lock — so the one signal is a running `claude` naming it, by id or by name,
/// in whole arguments of processes that really are `claude`; see
/// [`words_of_claude_processes`].
///
/// Freshness is NOT a second signal: it made every conversation this console had
/// just stopped look busy for two minutes. The remaining risk is a session
/// started outside the console whose command line names neither.
pub fn in_use(conversation: &Conversation, running: &Running) -> bool {
    // Could not ask, so cannot say it is free. Held busy is visible and recoverable;
    // the other direction puts two processes on one transcript.
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

/// What the process table said, or that it could not be asked. Two states,
/// because an empty answer is never evidence that nothing is running.
pub enum Running {
    /// `ps` answered. Empty means nothing is running, which is a real answer.
    Asked(Vec<String>),
    /// The question could not be put at all.
    Unasked,
}

/// Every argument of every `claude` this user is running, as separate words.
/// Shelled out rather than a crate. Failing to ask is [`Running::Unasked`], never
/// an empty list.
fn arguments() -> Running {
    let Ok(user) = std::env::var("USER") else {
        // Loud, because silence was the defect. launchd does inject `USER`.
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

/// The words of the command lines that actually ARE `claude`: every shell Claude
/// Code spawns sources a snapshot under `~/.claude/`, so `grep utterance` held a
/// session called `utterance` as in use. The first word's last path element
/// decides.
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

/// How much of a transcript's end to replay when picking it up. Generous: a
/// conversation cut off mid-tool-call is worse than one starting a little early.
const REPLAY_BYTES: u64 = 512 * 1024;

/// And how many events of it to keep: one tool call can be most of a megabyte,
/// so bytes alone say little about how much conversation was recovered.
const REPLAY_EVENTS: usize = 400;

/// When this session last did anything, from the transcript it is writing — the
/// question the list actually asks. `started` is when this console picked the
/// process up, which for a long conversation is hours off. `None` rather than
/// zero when there is no transcript.
pub fn touched(root: &Path, id: &str) -> Option<u64> {
    Some(about(root, id)?.touched)
}

/// What a live session's transcript says about it, in one read: one `stat` and
/// one tail read for all three, taken at one moment over a file being appended to.
#[derive(Debug)]
pub struct About {
    pub name: Option<String>,
    /// See [`Conversation::modified`] — the same quantity, decided the same way.
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

/// When this conversation last did anything. Not the file's date: picking a
/// conversation up appends `mode`, `permission-mode` and `bridge-session` lines,
/// none of them anything anybody said, and a list dated by the file said `just
/// now` about one nobody had spoken to. So: the last line that IS conversation,
/// which [`crate::protocol::read_recorded`] tells apart; the file's date only when
/// the tail holds none.
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

/// What a conversation is made of, for something that has to read it without
/// being in it. See [`crate::gist`].
#[derive(Debug, Default)]
pub struct Material {
    /// The first real instruction in the file, when one can be found inside
    /// [`BYTES_TO_FIND_CWD`].
    pub opening: Option<String>,
    /// The last few things said, newest last, each already labelled with who
    /// said it.
    pub recent: Vec<String>,
}

/// How much of one line of conversation to keep: a summariser needs the shape,
/// and one pasted stack trace would otherwise be the whole budget.
const LINE: usize = 400;

/// Read a conversation down to what it is about: prompts and replies only, since
/// tool calls are most of the bytes and almost none of the subject. Both ends,
/// because the opening says what it was set up to do and the last exchanges what
/// it has become.
pub fn material(path: &Path, keep: usize) -> Material {
    use std::io::{BufRead, Read};

    let mut found = Material::default();
    if let Ok(file) = std::fs::File::open(path) {
        for line in std::io::BufReader::new(file.take(BYTES_TO_FIND_CWD)).lines() {
            let Ok(line) = line else { break };
            // `read_recorded` already drops the plumbing a transcript opens with, so its
            // first `Prompt` is the first thing a person said.
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

/// How many times someone has spoken to this session since it was last
/// compacted — exchanges, not messages, and counted from the file so a resume or
/// an upgrade does not reset it. Read forward from where the last count stopped:
/// a whole-file pass at the end of every turn left the console deaf to its own
/// session on gigabyte files.
#[derive(Debug, Clone, Copy, Default, Serialize, serde::Deserialize)]
pub struct Counted {
    /// Exchanges since the last compaction.
    pub interactions: u32,
    /// How far into the file that answer accounts for, in bytes — always a line
    /// boundary.
    pub through: u64,
}

/// What the bytes appended to a transcript since last time turned out to hold:
/// three questions off one read.
#[derive(Debug, Clone, Default)]
pub struct Appended {
    pub counted: Counted,
    /// Background tasks the harness reported finished, by whichever name the
    /// notification gave — see [`crate::protocol::Named`]. The only way a live
    /// session finds out: the notification is a user message the CLI writes to the
    /// transcript and does NOT replay on stdout.
    pub finished: Vec<crate::protocol::Named>,
    /// Whether a compaction was filed among these bytes — again the only way a
    /// running session finds out; see [`crate::protocol::Event::Compacted`].
    pub compacted: bool,
    /// The newest fullness these bytes recorded, and `None` when a compaction came
    /// after. Read only alongside [`Self::compacted`]; otherwise the live stream is
    /// the better source.
    pub context: Option<u64>,
}

/// A place in a transcript somebody might want to come back to: things they said,
/// pictures they sent, and where the conversation was cut.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Landmark {
    /// Where to ask for it, as a byte offset — the END of the line carrying it, since
    /// `page` reads backwards from a cursor and a cursor at the start would return the
    /// page that stops just short. The same cursor `/api/sessions/{id}/earlier`
    /// takes; not a position anybody can be shown, since one picture is kilobytes on
    /// a line.
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
    /// A picture they sent, by the name of the copy kept for it.
    Shown,
    /// Where the conversation was cut and started again.
    Compacted,
}

/// How much of a landmark to carry: shorter than [`LINE`], a strip of things to tap.
const SIGN: usize = 120;

/// Every landmark in a transcript, oldest first.
///
/// This reads and parses the WHOLE file, seconds on a large one; call it off the
/// executor. No byte-level prescan: the first `"type":"` in a line is often a
/// nested one, and a user message's `content` is often a bare string with no typed
/// block, so a prescan silently misses most landmarks.
pub fn landmarks(path: &Path) -> Vec<Landmark> {
    landmarks_from(path, 0).found
}

/// What one walk read, and the byte the next one should start at.
#[derive(Debug, Clone, Default)]
pub struct Walk {
    pub found: Vec<Landmark>,
    /// One past the last COMPLETE line read. Never the file's length — see
    /// [`landmarks_from`].
    pub through: u64,
}

/// The same walk, starting at a byte already read. `from` must be a line
/// boundary, and the only honest source is a previous walk's [`Walk::through`].
/// A half-written last line is not read and `through` stops before it: taking
/// the file's length would restart the next walk mid-line and lose that landmark.
/// A `from` past the end yields nothing.
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
                crate::protocol::Event::Shown { name } => (Mark::Shown, name),
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

/// One line of a landmark, cut to [`SIGN`]. Newlines go first: a pasted message
/// is one landmark.
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

/// Where a seed can start counting without changing what it counts.
///
/// Exact, not close: [`counted`] resets `interactions` and drops the context at a
/// compaction, so starting AT the last boundary reaches the same state having read
/// a thousandth of the bytes. Searched backwards in widening windows, WITH the
/// parser — the boundary is whatever [`crate::protocol::read_recorded`] calls a
/// compaction. 0 when none is within [`COMPACTION_LIMIT`].
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
        // `by_ref`, because the span may have to widen and read again.
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
            // The first line of a mid-file chunk is a fragment, and one that happens to
            // parse is worse than one that does not.
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

/// Count what has been appended since `so_far` was true. Stops before a partial
/// last line, or a truncated object reads as nothing and is never looked at again.
/// A file that has shrunk was replaced, so the count starts again.
pub fn counted(path: &Path, so_far: Counted) -> Appended {
    use std::io::{BufRead, Seek, SeekFrom};

    let mut finished = Vec::new();
    let mut compacted = false;
    let mut context = None;
    let Ok(mut file) = std::fs::File::open(path) else {
        return Appended {
            counted: so_far,
            finished,
            compacted: false,
            context: None,
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
        for event in crate::protocol::read_recorded(&String::from_utf8_lossy(&line)) {
            match event {
                crate::protocol::Event::Compacted => {
                    found.interactions = 0;
                    compacted = true;
                    // What was measured before the boundary described a conversation that has just
                    // stopped existing.
                    context = None;
                }
                // Kept only for the compaction case: these bytes may hold a request made AFTER
                // the boundary.
                crate::protocol::Event::Context { tokens } => context = Some(tokens),
                crate::protocol::Event::Prompt { .. } => found.interactions += 1,
                // The only place a live session learns that background work has ended — the
                // harness files it as a user message the CLI does not put on stdout; see
                // [`Appended::finished`].
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
    }
}

/// The transcript file for a session id, wherever Claude Code filed it. Searched
/// rather than computed: the encoding is undocumented.
pub fn transcript_of(root: &Path, id: &str) -> Option<PathBuf> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten())
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        // The extension is what makes this a transcript; the stem alone matches the
        // directory beside it. See [`reader::transcript::is_transcript`].
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

/// Every event one transcript line carries, each wearing that line's time: a line
/// can hold several events, and they share the time the file recorded.
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
/// `before` is a cursor from a previous [`Page`], or `None` for the newest. A
/// cursor, not a count: the file grows, and a client holds FOLDED entries, so a
/// count never meant what the server read it as. Re-read each time rather than
/// kept; the span doubles until it has a page's worth.
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
        // `by_ref`: `take` consumes the reader and the span may have to widen.
        if file
            .by_ref()
            .take(end - start)
            .read_to_end(&mut buf)
            .is_err()
        {
            return empty;
        }

        // Line starts as absolute offsets. Bytes, not a decoded string: lossy decoding
        // can change lengths, and an offset off by one names the middle of a line.
        let mut lines: Vec<(u64, &[u8])> = Vec::new();
        let mut at = start;
        for line in buf.split_inclusive(|byte| *byte == b'\n') {
            lines.push((at, line));
            at += line.len() as u64;
        }
        // The first line of a mid-file chunk is a fragment; one that happens to parse is
        // worse than one that does not.
        if start > 0 && !lines.is_empty() {
            lines.remove(0);
        }

        // Backwards from the newest, taking whole lines: the cursor has to name a line
        // boundary.
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

        // Ran out of buffer with more file behind it: widen, rather than return a short
        // page that reads as "this is all there is".
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

/// Where the last whole line of a transcript ends: a cursor that names no line
/// still being written.
pub fn written(path: &Path) -> u64 {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| bytes.iter().rposition(|byte| *byte == b'\n'))
        .map_or(0, |at| at as u64 + 1)
}

/// What a transcript has gained since `from`, a cursor from [`written`] or from a
/// previous call: the events of every whole line after it, and the cursor past them.
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
        // Filled in by `conversations`, which reads the process table once for the list.
        busy: false,
    })
}

/// The working directory a transcript records for itself: the first line that
/// carries one, read forward, capped by [`BYTES_TO_FIND_CWD`] — which also bounds
/// what one `file-history-snapshot` line can cost.
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

/// What the end of a transcript says about the conversation as it stands. Both
/// facts are "the last one wins" over the same bytes: one seek, one pass.
#[derive(Debug, Default)]
struct Tail {
    /// See [`Conversation::name`].
    name: Option<String>,
    /// See [`Conversation::context`].
    context: Option<u64>,
    /// When the last thing anybody SAID was said, in epoch milliseconds. See
    /// [`Conversation::modified`].
    spoke: Option<i64>,
}

/// The first thing this session was asked to do, from the head of its transcript.
///
/// Derived, never carried: the head of an append-only file does not move. The
/// first PROMPT, not the first line — `read_recorded` declines the plumbing a
/// transcript opens with.
///
/// A compacted session has no origin in this file, and says so: the first user
/// text is the harness's `This session is being continued…` preamble, refused
/// here. `None` is the right answer; a recent prompt in its place would be a false
/// claim about where the conversation began.
pub fn opening(path: &Path) -> Option<String> {
    use std::io::{BufRead, BufReader, Read};

    let file = std::fs::File::open(path).ok()?;
    // Enough for the opening exchange: a compacted transcript opens with its whole
    // summary, which can put the first user line hundreds of kilobytes in.
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

/// How a transcript opens when it is not the beginning of the conversation —
/// matched on the harness's own words; see [`opening`].
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

    // Keyed by the field the line declares, so the precedence is decided by the
    // shared order.
    let mut names: std::collections::BTreeMap<&'static str, String> =
        std::collections::BTreeMap::new();
    for line in String::from_utf8_lossy(&tail).lines() {
        let events = crate::protocol::read_recorded(line);
        // A line that carries a conversation event is a line somebody said, and
        // `read_recorded` yields nothing for the metadata — so the last line it accepts
        // is the last thing that actually happened.
        if !events.is_empty()
            && let Some(at) = crate::protocol::recorded_at(line)
        {
            found.spoke = Some(at);
        }
        for event in events {
            match event {
                crate::protocol::Event::Context { tokens } => found.context = Some(tokens),
                // Everything measured above the boundary belongs to a conversation that no
                // longer exists. Read forward, so a request since the compaction takes it back.
                crate::protocol::Event::Compacted => found.context = None,
                _ => {}
            }
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // Field names from the shared vocabulary, so this and the viewer cannot drift.
        // Read forward, so the last of each kind wins.
        for line in reader::transcript::AS_CONVERSATION {
            if let Some(name) = value.get(line.field).and_then(|v| v.as_str()) {
                names.insert(line.field, name.to_string());
            }
        }
    }
    // The conversation's order: the title a person last chose wins here, where
    // `/agents` prefers the agent name — see
    // [`reader::transcript::AS_CONVERSATION`] for both orders.
    found.name = reader::transcript::AS_CONVERSATION
        .iter()
        .find_map(|line| names.get(line.field))
        .filter(|name| !name.trim().is_empty())
        .cloned();
    found
}
