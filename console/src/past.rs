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
    /// When anything last happened in it, in milliseconds since the epoch.
    ///
    /// ⚠ **From the last line of the conversation, not from the file's date** —
    /// see [`last_moved`]. Picking a transcript up writes to it without anybody
    /// saying anything, so the two differ by exactly the gap this console kept
    /// getting wrong.
    pub modified: u64,
    /// How much was said. A rough weight, and the cheap one: counting turns means
    /// reading the whole file, and these reach tens of megabytes.
    pub bytes: u64,
    /// What the conversation calls itself — `music`, `health` — or none when it
    /// never took a name. A hex prefix identifies a transcript; only this
    /// identifies the *work*.
    pub name: Option<String>,
    /// How full the context was at the last request the transcript records, in
    /// tokens — the same quantity a running session reports for itself.
    ///
    /// ⚠ **No window to divide it by.** The size of the context window is
    /// declared on the result line, which lives on the CLI's stdout and never in
    /// the file, so a conversation that is not running can say how full it is and
    /// not what it is full of. The client shows the count alone for these.
    ///
    /// `None` when the tail read finds no assistant message — a conversation
    /// that ended on a large tool result can push the last one out of
    /// [`TAIL_BYTES`], and no number is the honest answer there.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,
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

/// Every id that has a transcript under `root`, filtered by nothing.
///
/// ⚠ **Deliberately not [`conversations`].** That one is a display list: it
/// leaves out anything that ran from a temporary directory, and reads each file
/// to learn what it is. This is the plain question of which conversations exist,
/// asked by the housekeeping that deletes things — see [`crate::images::tidy`],
/// where using the display list instead would throw away the pictures belonging
/// to a conversation that was only ever hidden.
pub fn transcript_ids(root: &Path) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .flat_map(|project| std::fs::read_dir(project.path()).into_iter().flatten())
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        // The extension, for the reason [`transcript_of`] gives at length: there
        // is a directory beside each transcript with the same name.
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
/// Cheap despite the file being enormous: [`tail_of`] reads the last
/// [`TAIL_BYTES`] and no more.
/// When this session last did anything, from the transcript it is writing.
///
/// ⚠ **The question the list actually asks.** A session's `started` is when this
/// console picked the process up — carried across an in-place upgrade, reset by a
/// restart — and for a long conversation the two are nothing like each other:
/// the console's own session showed `13h ago` on a card whose transcript had been
/// written to four seconds earlier. Every turn appends, so the file's modification
/// time is the last moment anything happened, and it is the same quantity a
/// conversation on disk reports — which is what lets one column mean one thing.
///
/// `None` rather than zero when there is no transcript: a missing date is a
/// thing a client can decline to render, where the epoch is a date it would
/// render as half a century ago.
pub fn touched(root: &Path, id: &str) -> Option<u64> {
    Some(about(root, id)?.touched)
}

/// What a live session's transcript says about it, in one read.
///
/// The roster wants all three for every session on every poll, and they come off
/// one `stat` and one tail read between them — see [`tail_of`]. Asking
/// separately would be three passes over the same bytes and, worse, three
/// answers taken at three different moments over a file that is being appended
/// to.
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

/// When this conversation last did anything.
///
/// ⚠ **Not the file's own date, and the difference is not academic.** Picking a
/// conversation up appends to it: `mode`, `permission-mode` and `bridge-session`
/// lines go in at the moment of resume, none of them anything anybody said.
/// Measured on `scanner`, opened after two days: the file was stamped that
/// second, while the last line carrying a timestamp was `2026-08-03T16:40:53Z`.
/// A list dated by the file said `just now` about a conversation nobody had
/// spoken to.
///
/// So the date comes from the last line of the transcript that *is* a
/// conversation — which [`crate::protocol::read_recorded`] already knows how to
/// tell apart, since it yields nothing for the metadata. The file's own date is
/// the fallback for a transcript whose tail holds no such line at all, which a
/// conversation ending in a very large tool result can manage.
///
/// ⚠ An earlier version of this compared the file's *size* against what it was
/// when the session picked it up, on the reasoning that nothing is said without
/// being appended. True, and not enough: things nobody said are appended too,
/// which is exactly what those three lines are.
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

/// How much of one line of conversation to keep.
///
/// A summariser needs the shape of what was said, not the whole of it, and one
/// pasted stack trace would otherwise be the entire budget.
const LINE: usize = 400;

/// Read a conversation down to what it is about.
///
/// ⚠ **Prompts and replies only.** Tool calls and their results are most of the
/// bytes in any working transcript and almost none of the subject — a hundred
/// `Bash` lines say a build was run, not what it was for. Dropping them is what
/// makes a few thousand characters enough.
///
/// Both ends, because they answer different halves of "what is this": the
/// opening says what it was set up to do, and the last few exchanges say what it
/// has become. A conversation drifts, so neither is enough alone.
pub fn material(path: &Path, keep: usize) -> Material {
    use std::io::{BufRead, Read};

    let mut found = Material::default();
    if let Ok(file) = std::fs::File::open(path) {
        for line in std::io::BufReader::new(file.take(BYTES_TO_FIND_CWD)).lines() {
            let Ok(line) = line else { break };
            // `read_recorded` already drops the plumbing a transcript opens with
            // — the command echoes, the caveats, the local-command output — so
            // the first `Prompt` it yields is the first thing a person actually
            // said. Reproducing that filter here would be a second copy of it.
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
/// ⚠ **Read forward from where the last count stopped, not from the start.**
/// This was a whole-file pass at the end of every turn, and the files are not
/// small: 2.1 GB and 267,002 lines for the largest here, twenty-four seconds
/// just to read the bytes and a serde parse per line on top. It ran in the task
/// that reads that session's stdout, so every turn ended with the console going
/// deaf to its own session for as long as it took to count something that had
/// changed by one.
///
/// The offset is what makes a running total honest: the count is still derived
/// from the file, so a compaction the CLI performs and announces nowhere is
/// still seen — it is simply seen by reading the few kilobytes that arrived
/// rather than the two gigabytes that did not.
#[derive(Debug, Clone, Copy, Default, Serialize, serde::Deserialize)]
pub struct Counted {
    /// Exchanges since the last compaction.
    pub interactions: u32,
    /// How far into the file that answer accounts for, in bytes — always a line
    /// boundary, so the next read starts on a whole line.
    pub through: u64,
}

/// What the bytes appended to a transcript since last time turned out to hold.
///
/// Three questions off one read, because they want the same few kilobytes and
/// the file is measured in gigabytes.
#[derive(Debug, Clone, Default)]
pub struct Appended {
    pub counted: Counted,
    /// Background tasks the harness reported finished, by the id of the call
    /// that started each — the same id [`crate::protocol::Running::Began`]
    /// carries.
    ///
    /// ⚠ **This is the only way a live session ever finds out.** A backgrounded
    /// call returns at once with a task id, so its notification is the sole
    /// end-of-work signal — and the notification is injected as a user message
    /// nobody typed, which the CLI writes to the transcript and does **not**
    /// replay on stdout. Measured 2026-08-06: every `Background` event this
    /// console had ever shown came from a seed replaying the file, and a task
    /// that finished in 75 seconds sat on the front page for 26 minutes.
    pub finished: Vec<String>,
    /// Whether a compaction was filed among these bytes.
    ///
    /// ⚠ **The only way a running session finds out.** The CLI writes the
    /// boundary to the transcript and says nothing on stdout, so a console
    /// watching the stream sees a compaction as silence — see
    /// [`crate::protocol::Event::Compacted`]. Until it knows, it goes on showing
    /// how full a conversation was that no longer exists.
    pub compacted: bool,
    /// The newest fullness these bytes recorded, and `None` when a compaction
    /// came after the last of them.
    ///
    /// Only meaningful alongside [`Self::compacted`], and read only then: at any
    /// other time the live stream is the better source, because it arrives per
    /// message rather than per turn.
    pub context: Option<u64>,
}

/// Count what has been appended since `so_far` was true.
///
/// `so_far.through` of 0 is the whole file, which is what a seed does once.
///
/// ⚠ **Stops before a partial last line.** A transcript is appended to while
/// this runs, so the tail can be half a line; counting it would read a truncated
/// JSON object as nothing and then never look at it again, losing the exchange
/// it recorded. The offset returned is the end of the last *complete* line.
///
/// A file that has shrunk is one that was replaced, so the count starts again:
/// an offset into a file that no longer exists would land mid-line at best.
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
                    // What was measured before the boundary described a
                    // conversation that has just stopped existing.
                    context = None;
                }
                // Kept only for the compaction case: these bytes may hold a
                // request made *after* the boundary, and that one is the answer.
                // Otherwise the live stream is the better source — it arrives
                // per message where this arrives per turn.
                crate::protocol::Event::Context { tokens } => context = Some(tokens),
                crate::protocol::Event::Prompt { .. } => found.interactions += 1,
                // ⚠ **The only place a live session learns that background work
                // has ended.** The harness files the notification as a user
                // message nobody typed, and the CLI does not put it on stdout —
                // measured, and see [`Appended::finished`] — so the reader of the
                // stream never sees one. It is in the file, in the same few
                // kilobytes this is already reading for the count.
                crate::protocol::Event::Background { tool, .. } => finished.push(tool),
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
        // ⚠ **The extension is what makes this a transcript**, and matching on
        // the stem alone opens the directory Claude Code puts beside the file as
        // if it were the conversation. The whole account is in
        // [`reader::transcript::is_transcript`], which the viewer calls too —
        // this bug existed here because the two crates knew it separately.
        .filter(|path| reader::transcript::is_transcript(path))
        .find(|path| path.file_stem().and_then(|stem| stem.to_str()) == Some(id))
}

/// The directory one conversation was working in.
///
/// By id, without listing anything: a caller that wants one conversation's
/// directory must not pay for every conversation on the machine, which is what
/// filtering [`conversations`] would cost.
pub fn dir_of(root: &Path, id: &str) -> Option<String> {
    cwd_of(&transcript_of(root, id)?)
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

/// What the end of a transcript says about the conversation as it stands.
///
/// Both facts here are "the last one wins" over the same bytes, which is why
/// they are read together: one seek, one buffer, one pass.
#[derive(Debug, Default)]
struct Tail {
    /// See [`Conversation::name`].
    name: Option<String>,
    /// See [`Conversation::context`].
    context: Option<u64>,
    /// When the last thing anybody *said* was said, in epoch milliseconds. See
    /// [`Conversation::modified`] for why the file's own date will not do.
    spoke: Option<i64>,
}

/// Read the tail of a transcript for what it says about itself.
///
/// **The name.** Two line types carry it: `custom-title` (what it was
/// deliberately called) and `agent-name`. The first wins where both exist,
/// because one is a decision and the other is a default.
///
/// **The fullness.** Every assistant message records the tokens its request
/// carried, so the last one in the file is how full the conversation was when it
/// stopped — read through [`crate::protocol::read_recorded`] rather than off the
/// JSON here, so that "input plus cache-creation plus cache-read" is stated in
/// exactly one place.
///
/// Read from the **end** — see [`TAIL_BYTES`] — because both answers are about
/// the conversation now: a session is renamed as its job changes, and a context
/// that was full an hour before a compaction is not the number anybody wants. A
/// chunk taken from an arbitrary byte offset starts mid-line and possibly
/// mid-character, so the first line is expected to be rubbish and unparseable
/// lines are skipped rather than treated as the end of the file.
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

    // Keyed by the field the line declares, so the precedence below is decided by
    // the shared order and never by which local happens to be checked first.
    let mut names: std::collections::BTreeMap<&'static str, String> =
        std::collections::BTreeMap::new();
    for line in String::from_utf8_lossy(&tail).lines() {
        let events = crate::protocol::read_recorded(line);
        // ⚠ **A line that carries a conversation event is a line somebody said**
        // — and `read_recorded` is where that distinction already lives: it
        // yields nothing for the metadata a transcript is full of. So the last
        // line it accepts is the last thing that actually happened, whatever the
        // CLI has appended since.
        if !events.is_empty()
            && let Some(at) = crate::protocol::recorded_at(line)
        {
            found.spoke = Some(at);
        }
        for event in events {
            match event {
                crate::protocol::Event::Context { tokens } => found.context = Some(tokens),
                // Everything measured above the boundary was replaced by a
                // summary, so the last figure is not stale — it belongs to a
                // conversation that no longer exists. Read forward, so a request
                // made since the compaction takes the field back.
                crate::protocol::Event::Compacted => found.context = None,
                _ => {}
            }
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        // Field names from the shared vocabulary, so this and the viewer cannot
        // drift on the spelling of a line they both read. Read forward, so the
        // last of each kind in the tail is the one kept.
        for line in reader::transcript::AS_CONVERSATION {
            if let Some(name) = value.get(line.field).and_then(|v| v.as_str()) {
                names.insert(line.field, name.to_string());
            }
        }
    }
    // ⚠ **The conversation's order, and the viewer deliberately uses the other
    // one.** This names a conversation in a list somebody picks from, so the
    // title a person last chose wins; `/agents` asks who did the work and
    // prefers the agent name. That is the CLI's own split — see
    // [`reader::transcript::AS_CONVERSATION`], which sets out both orders and
    // the two chains in the CLI they come from.
    found.name = reader::transcript::AS_CONVERSATION
        .iter()
        .find_map(|line| names.get(line.field))
        .filter(|name| !name.trim().is_empty())
        .cloned();
    found
}
