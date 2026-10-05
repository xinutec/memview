//! What a `Bash` call will change in the files it writes, as the reader predicts
//! it — and, once it has run, whether it did.
//!
//! The IO edges of [`reader::predict`]. A `PreToolUse` hook hands every command to
//! [`Edits::before`]: the files the prediction depends on are read and passed in,
//! and what comes back is drawn at once as hunks. The `PostToolUse` hook calls
//! [`Edits::finished`], which reads the predicted files again and asks
//! [`reader::predict::check`] whether they hold what was predicted — or, for a
//! call sent to the background, leaves that to [`Edits::ended`]. A divergence
//! is a defect in the evaluator, kept in full so it can become a test — see
//! `docs/evaluator.md`, "The live oracle". A file predicted
//! only on the condition that an unknown program left it alone is checked the
//! same way, but its divergence is kept apart: either may be wrong. Every file
//! checked, and every prediction never checked, leaves an outcome row.
//!
//! Nothing here runs the command or writes a file it names.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use reader::predict::{
    Alternatives, Conditional, Dirs, Divergence, Files, Sight, Written, check, predict,
};
use serde::{Deserialize, Serialize};
use similar::TextDiff;

/// One changed region of one file, with its context lines on both sides.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Hunk {
    pub path: String,
    pub before: String,
    pub after: String,
    /// Present when the file is predicted only if these programs left it alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub assumed: Option<Assumed>,
    /// Present when the file will be one of several texts, and this is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub alternative: Option<Alternative>,
}

/// Which of a file's possible texts a hunk draws: an `if` the reader could not
/// decide leaves it one of several.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Alternative {
    /// Counted from 1.
    pub at: usize,
    pub of: usize,
}

/// The unknown programs a conditional prediction assumes did not touch its
/// file: those run before its last write, which may have changed what the
/// write read, and those run after it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Assumed {
    pub before: Vec<String>,
    pub after: Vec<String>,
}

impl From<&reader::predict::Assumed> for Assumed {
    fn from(assumed: &reader::predict::Assumed) -> Self {
        Self {
            before: assumed.before.clone(),
            after: assumed.after.clone(),
        }
    }
}

/// What one call is predicted to change, as stored and as sent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Edited {
    pub call: String,
    pub hunks: Vec<Hunk>,
}

/// A call whose files did not end up as predicted, as the client is told it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Diverged {
    pub call: String,
    pub paths: Vec<String>,
}

/// What a conversation's calls were predicted to change, and which of them did not.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Record {
    pub edited: Vec<Edited>,
    pub diverged: Vec<Diverged>,
}

/// A call whose files did not end up as predicted: the finding, in full. It
/// carries what the evaluator was given, so `predict-report --live` can make the
/// prediction again under a later evaluator and say whether it still diverges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub session: String,
    pub call: String,
    pub command: String,
    pub cwd: String,
    /// The files the prediction was made from, as they were before the call.
    pub files: Files,
    /// The directories it listed, likewise.
    #[serde(default, skip_serializing_if = "Dirs::is_empty")]
    pub dirs: Dirs,
    pub path: String,
    pub predicted: Option<String>,
    pub actual: Option<String>,
    /// For a file predicted one of several texts: all of them, none of which
    /// it held.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<Option<String>>,
}

/// A conditional prediction that diverged: the finding, and what it assumed.
/// Not a finding yet — the program assumed harmless may have made the change.
#[derive(Serialize)]
struct Doubted<'a> {
    #[serde(flatten)]
    finding: &'a Finding,
    assumed: &'a Assumed,
}

/// What became of one predicted file: `agreed`, `diverged`, or `unchecked`
/// with why. Kept for good: what a file held after its call cannot be looked
/// at again, and the rate of each is how the evaluator and its assumptions are
/// judged.
#[derive(Serialize)]
struct Outcome<'a> {
    at: String,
    session: &'a str,
    call: &'a str,
    path: &'a str,
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    why: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    assumed: Option<&'a Assumed>,
    /// For a file predicted one of several texts: how many.
    #[serde(skip_serializing_if = "Option::is_none")]
    of: Option<usize>,
}

/// Larger files are not read: a hook waits on this.
const LARGEST: u64 = 2 * 1024 * 1024;

/// Lines of unchanged text kept either side of a change.
const CONTEXT: usize = 3;

/// Where predictions and findings go. Overridable because this writes.
pub fn edits_root() -> PathBuf {
    if let Ok(set) = std::env::var("CONSOLE_EDIT_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".console")
        .join("edits")
}

/// What a live call's prediction did not follow, by the census's names, with
/// what the evaluator was given: `predict-report --live` predicts again under the
/// current evaluator rather than counting names an older one wrote.
#[derive(Serialize)]
struct Refused {
    session: String,
    call: String,
    /// When, so the rows can be pruned: a refusal is recomputable from the
    /// transcript and only recent ones rank.
    at: String,
    command: String,
    cwd: String,
    files: Files,
    #[serde(skip_serializing_if = "Dirs::is_empty")]
    dirs: Dirs,
    /// Files it did predict, beside the refusals.
    predicted: usize,
    /// Files predicted on the condition that an unknown program left them alone.
    conditional: usize,
    refused: Vec<String>,
}

/// A prediction waiting for its call to finish.
struct Pending {
    session: String,
    command: String,
    cwd: String,
    files: Files,
    dirs: Dirs,
    written: Vec<Written>,
    conditional: Vec<Conditional>,
    alternatives: Vec<Alternatives>,
    since: std::time::Instant,
    /// For a backgrounded call, its files when it was sent to the background: a
    /// later edit can change them before its task ends.
    early: Option<Files>,
}

/// How long a prediction waits for its call to end. A backgrounded call in a
/// session the console does not run never reports its end here.
const HELD: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// How long a refusal row is kept. Nothing in it is golden: the command is in
/// the transcript, and the ranking wants recent traffic.
const REFUSALS_KEPT: time::Duration = time::Duration::days(30);

/// How often the refusal rows are pruned, at most.
const PRUNE_EVERY: std::time::Duration = std::time::Duration::from_secs(60 * 60);

#[derive(Default)]
pub struct Edits {
    root: PathBuf,
    pending: Mutex<HashMap<String, Pending>>,
    /// The refusal file's lock, holding when it was last pruned.
    pruned: Mutex<Option<std::time::Instant>>,
}

impl Edits {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            pending: Mutex::default(),
            pruned: Mutex::default(),
        }
    }

    /// Keep a refusal row, then drop rows older than [`REFUSALS_KEPT`] and any
    /// without a date, at most once per [`PRUNE_EVERY`]. Under one lock: the
    /// hook runs on a blocking pool, and a row appended while another thread
    /// rewrites the file would be lost.
    fn keep_refusal(&self, refused: &Refused) -> std::io::Result<()> {
        let mut pruned = self.pruned.lock();
        let file = self.root.join("refused.jsonl");
        self.keep(&file, refused)?;
        if pruned.is_some_and(|last| last.elapsed() < PRUNE_EVERY) {
            return Ok(());
        }
        *pruned = Some(std::time::Instant::now());
        let text = std::fs::read_to_string(&file)?;
        let oldest = time::OffsetDateTime::now_utc() - REFUSALS_KEPT;
        let kept: Vec<&str> = text
            .lines()
            .filter(|line| {
                serde_json::from_str::<serde_json::Value>(line)
                    .ok()
                    .and_then(|row| {
                        time::OffsetDateTime::parse(
                            row["at"].as_str()?,
                            &time::format_description::well_known::Rfc3339,
                        )
                        .ok()
                    })
                    .is_some_and(|at| at >= oldest)
            })
            .collect();
        if kept.len() < text.lines().count() {
            std::fs::write(&file, kept.join("\n") + "\n")?;
        }
        Ok(())
    }

    /// Predict what `command` will change, before it runs. `None` when it writes
    /// nothing the reader can follow.
    pub fn before(&self, session: &str, call: &str, command: &str, cwd: &str) -> Option<Edited> {
        let script = reader::syntax::parse(command).ok()?;
        let home = std::env::var("HOME").unwrap_or_default();
        let seen = Seen::default();
        let prediction = predict(&script, cwd, &home, &seen);
        let (mut files, dirs) = (seen.files.into_inner(), seen.dirs.into_inner());
        named_files(command, cwd, &home, &mut files);
        if !prediction.unfollowed.is_empty() {
            let refused = Refused {
                session: session.to_string(),
                call: call.to_string(),
                at: now(),
                command: command.to_string(),
                cwd: cwd.to_string(),
                files: files.clone(),
                dirs: dirs.clone(),
                predicted: prediction.written.len(),
                conditional: prediction.conditional.len(),
                refused: prediction
                    .unfollowed
                    .iter()
                    .map(|unfollowed| unfollowed.why.census_name())
                    .collect(),
            };
            if let Err(why) = self.keep_refusal(&refused) {
                tracing::warn!("could not keep the refusals of {call}: {why}");
            }
        }
        if prediction.written.is_empty()
            && prediction.conditional.is_empty()
            && prediction.alternatives.is_empty()
        {
            return None;
        }
        let sure = prediction.written.iter().map(|written| (written, None));
        let unsure = prediction
            .conditional
            .iter()
            .map(|file| (&file.written, Some(Assumed::from(&file.assumed))));
        // The text each file holds now, for the diff's first side.
        let hunks = sure
            .chain(unsure)
            .flat_map(|(written, assumed)| {
                let now = read(Path::new(&written.path)).flatten().unwrap_or_default();
                // A file the command removes shows as every line of it deleted.
                let mut drawn = hunks(
                    &written.path,
                    &now,
                    written.text.as_deref().unwrap_or_default(),
                );
                for hunk in &mut drawn {
                    hunk.assumed = assumed.clone();
                }
                drawn
            })
            .chain(prediction.alternatives.iter().flat_map(|set| {
                let now = read(Path::new(&set.path)).flatten().unwrap_or_default();
                let of = set.texts.len();
                set.texts.iter().enumerate().flat_map(move |(at, text)| {
                    let mut drawn = hunks(&set.path, &now, text.as_deref().unwrap_or_default());
                    for hunk in &mut drawn {
                        hunk.alternative = Some(Alternative { at: at + 1, of });
                    }
                    drawn
                })
            }))
            .collect();
        let edited = Edited {
            call: call.to_string(),
            hunks,
        };
        let mut pending = self.pending.lock();
        let expired: Vec<String> = pending
            .iter()
            .filter(|(_, waiting)| waiting.since.elapsed() >= HELD)
            .map(|(call, _)| call.clone())
            .collect();
        for call in expired {
            // A call that failed never reports its end here.
            if let Some(waiting) = pending.remove(&call) {
                self.unchecked(&call, &waiting, "never finished");
            }
        }
        pending.insert(
            call.to_string(),
            Pending {
                session: session.to_string(),
                command: command.to_string(),
                cwd: cwd.to_string(),
                files,
                dirs,
                written: prediction.written,
                conditional: prediction.conditional,
                alternatives: prediction.alternatives,
                since: std::time::Instant::now(),
                early: None,
            },
        );
        drop(pending);
        if let Err(why) = self.keep(&self.file(session), &edited) {
            tracing::warn!("could not keep the prediction of {call}: {why}");
        }
        Some(edited)
    }

    /// The call's hook says it is done. A call sent to the background has only
    /// started, so its check waits for [`Self::ended`], with a first look at its
    /// files kept for then; any other is checked now.
    pub fn finished(&self, call: &str, response: &serde_json::Value) -> Option<(String, Diverged)> {
        if response
            .get("backgroundTaskId")
            .is_some_and(serde_json::Value::is_string)
        {
            if let Some(pending) = self.pending.lock().get_mut(call) {
                pending.early = Some(look(&pending.paths()));
            }
            return None;
        }
        // A script that raised did not do what its text says, though the shell's
        // exit code can hide that behind a later command (`python3 … ; grep …`).
        // The interpreter's own marker is read where it puts it, at a line start.
        if raised(response) {
            if let Some(pending) = self.pending.lock().remove(call) {
                self.unchecked(call, &pending, "raised");
            }
            return None;
        }
        self.check(call)
    }

    /// A backgrounded call's task ended. One that did not complete did not do
    /// what its text says, so its prediction is dropped unchecked.
    pub fn ended(
        &self,
        call: &str,
        status: Option<&crate::protocol::Ended>,
    ) -> Option<(String, Diverged)> {
        match status {
            Some(crate::protocol::Ended::Completed) => self.check(call),
            _ => {
                if let Some(pending) = self.pending.lock().remove(call) {
                    self.unchecked(call, &pending, "did not complete");
                }
                None
            }
        }
    }

    /// Whether the call left its files as predicted: the files that did not, each
    /// kept as a finding. `None` when there was no prediction for the call.
    fn check(&self, call: &str) -> Option<(String, Diverged)> {
        let pending = self.pending.lock().remove(call)?;
        let all = pending.all();
        let seen = look(&pending.paths());
        let mut diverged = check(&all, &seen);
        // Either look holding the prediction agrees: a later edit may have
        // overtaken the first.
        if let Some(early) = &pending.early {
            let first = check(&all, early);
            diverged.retain(|late| first.iter().any(|d| d.path == late.path));
        }
        // A file predicted one of several texts agrees when it holds any.
        for set in &pending.alternatives {
            let Some(actual) = seen.get(&set.path) else {
                continue;
            };
            let holds = |actual: &Option<String>| set.texts.contains(actual);
            let early = pending
                .early
                .as_ref()
                .and_then(|early| early.get(&set.path));
            let agreed = holds(actual) || early.is_some_and(holds);
            self.outcome(&Outcome {
                at: now(),
                session: &pending.session,
                call,
                path: &set.path,
                outcome: if agreed { "agreed" } else { "diverged" },
                why: None,
                assumed: None,
                of: Some(set.texts.len()),
            });
            if !agreed {
                diverged.push(Divergence {
                    path: set.path.clone(),
                    predicted: None,
                    actual: actual.clone(),
                });
            }
        }
        for written in &all {
            let assumed = pending.assumed(&written.path);
            let outcome = if diverged.iter().any(|d| d.path == written.path) {
                "diverged"
            } else {
                "agreed"
            };
            self.outcome(&Outcome {
                at: now(),
                session: &pending.session,
                call,
                path: &written.path,
                outcome,
                why: None,
                assumed: assumed.as_ref(),
                of: None,
            });
        }
        for divergence in &diverged {
            let finding = Finding {
                session: pending.session.clone(),
                call: call.to_string(),
                command: pending.command.clone(),
                cwd: pending.cwd.clone(),
                files: pending.files.clone(),
                dirs: pending.dirs.clone(),
                path: divergence.path.clone(),
                predicted: divergence.predicted.clone(),
                actual: divergence.actual.clone(),
                alternatives: pending
                    .alternatives
                    .iter()
                    .find(|set| set.path == divergence.path)
                    .map(|set| set.texts.clone())
                    .unwrap_or_default(),
            };
            tracing::warn!("{call}: {} did not end up as predicted", divergence.path);
            let kept = match pending.assumed(&divergence.path) {
                Some(assumed) => self.keep(
                    &self.root.join("conditional.jsonl"),
                    &Doubted {
                        finding: &finding,
                        assumed: &assumed,
                    },
                ),
                None => self.keep(&self.root.join("findings.jsonl"), &finding),
            };
            if let Err(why) = kept {
                tracing::warn!("could not keep the finding for {call}: {why}");
            }
        }
        let diverged = Diverged {
            call: call.to_string(),
            paths: diverged.into_iter().map(|d: Divergence| d.path).collect(),
        };
        if !diverged.paths.is_empty()
            && let Err(why) = self.keep(&self.diverged_file(&pending.session), &diverged)
        {
            tracing::warn!("could not keep the divergence of {call}: {why}");
        }
        Some((pending.session, diverged))
    }

    /// Every prediction for `session`, and the calls whose files did not end up
    /// as predicted, oldest first.
    pub fn of(&self, session: &str) -> Record {
        Record {
            edited: lines(&self.file(session)),
            diverged: lines(&self.diverged_file(session)),
        }
    }

    /// An outcome row for each file a prediction named that was never checked.
    fn unchecked(&self, call: &str, pending: &Pending, why: &'static str) {
        for written in pending.all() {
            let assumed = pending.assumed(&written.path);
            self.outcome(&Outcome {
                at: now(),
                session: &pending.session,
                call,
                path: &written.path,
                outcome: "unchecked",
                why: Some(why),
                assumed: assumed.as_ref(),
                of: None,
            });
        }
        for set in &pending.alternatives {
            self.outcome(&Outcome {
                at: now(),
                session: &pending.session,
                call,
                path: &set.path,
                outcome: "unchecked",
                why: Some(why),
                assumed: None,
                of: Some(set.texts.len()),
            });
        }
    }

    /// Keeps one outcome row.
    fn outcome(&self, row: &Outcome<'_>) {
        if let Err(why) = self.keep(&self.root.join("outcomes.jsonl"), row) {
            tracing::warn!("could not keep the outcome of {}: {why}", row.call);
        }
    }

    fn diverged_file(&self, session: &str) -> PathBuf {
        self.file(session).with_extension("diverged.jsonl")
    }

    fn keep(&self, file: &Path, value: &impl Serialize) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let mut out = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file)?;
        writeln!(out, "{}", serde_json::to_string(value)?)
    }

    fn file(&self, session: &str) -> PathBuf {
        // A session id is a UUID; anything else must not become a path.
        let safe: String = session
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        self.root.join(format!("{safe}.jsonl"))
    }
}

impl Pending {
    /// Every file predicted, the certain ones first.
    fn all(&self) -> Vec<Written> {
        self.written
            .iter()
            .cloned()
            .chain(self.conditional.iter().map(|file| file.written.clone()))
            .collect()
    }

    /// Every path a check reads: the files predicted, and those predicted one
    /// of several texts.
    fn paths(&self) -> Vec<String> {
        self.all()
            .into_iter()
            .map(|written| written.path)
            .chain(self.alternatives.iter().map(|set| set.path.clone()))
            .collect()
    }

    /// What the prediction of `path` assumed, if it was conditional.
    fn assumed(&self, path: &str) -> Option<Assumed> {
        self.conditional
            .iter()
            .find(|file| file.written.path == path)
            .map(|file| Assumed::from(&file.assumed))
    }
}

/// This instant, as the rows date themselves.
fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .expect("RFC 3339 formats any instant")
}

/// Every value one JSON line each in `file`, skipping any that do not read.
fn lines<T: serde::de::DeserializeOwned>(file: &Path) -> Vec<T> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Sight from this disk, asked for as the evaluator reaches each file, keeping
/// what it showed: a finding or a refusal carries it, so the prediction can be
/// made again. Reads only — see `docs/evaluator.md`, "Sight".
#[derive(Default)]
struct Seen {
    files: std::cell::RefCell<Files>,
    dirs: std::cell::RefCell<Dirs>,
}

impl Sight for Seen {
    fn file(&self, path: &str) -> Option<Option<String>> {
        let shown = read(Path::new(path))?;
        self.files
            .borrow_mut()
            .insert(path.to_string(), shown.clone());
        Some(shown)
    }

    fn dir(&self, path: &str) -> Option<Option<Vec<String>>> {
        let shown = list(Path::new(path))?;
        self.dirs
            .borrow_mut()
            .insert(path.to_string(), shown.clone());
        Some(shown)
    }
}

/// The most text [`named_files`] keeps beside what the evaluator asked for.
const NAMED_KEPT: usize = 4 * 1024 * 1024;

/// Every file the command names, as the reconstruction reads it, added as it
/// is before the call to what the evaluator was shown: a later evaluator that
/// reaches further is replayed on them, rather than finding them unread. Kept
/// up to [`NAMED_KEPT`]; reads only.
fn named_files(command: &str, cwd: &str, home: &str, files: &mut Files) {
    let Ok(parsed) = reader::project::read(command) else {
        return;
    };
    let named = reader::shell_files::extract_knowing(&parsed, Some(cwd), home, &[]);
    let mut budget = NAMED_KEPT;
    for file in named.files {
        if files.contains_key(&file.path) {
            continue;
        }
        let Some(shown) = read(Path::new(&file.path)) else {
            continue;
        };
        let size = shown.as_ref().map_or(0, String::len);
        if size > budget {
            continue;
        }
        budget -= size;
        files.insert(file.path, shown);
    }
}

/// Larger directories are not listed: a hook waits on this.
const LARGEST_DIR: usize = 10_000;

/// A directory's names: `Some(None)` when it does not exist, `None` when it
/// cannot be listed here — not a directory, too large, or a name not UTF-8.
fn list(path: &Path) -> Option<Option<Vec<String>>> {
    let entries = match std::fs::read_dir(path) {
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Some(None),
        Err(_) => return None,
        Ok(entries) => entries,
    };
    let mut names = Vec::new();
    for entry in entries {
        if names.len() == LARGEST_DIR {
            return None;
        }
        names.push(entry.ok()?.file_name().into_string().ok()?);
    }
    Some(Some(names))
}

/// Whether a Python interpreter in the call reported an uncaught exception.
fn raised(response: &serde_json::Value) -> bool {
    ["stdout", "stderr"].iter().any(|stream| {
        response[stream]
            .as_str()
            .unwrap_or("")
            .lines()
            .any(|line| line.starts_with("Traceback (most recent call last):"))
    })
}

/// A file's text: `Some(None)` when it does not exist, `None` when it cannot be
/// read here — too large, not text, or not a regular file.
fn read(path: &Path) -> Option<Option<String>> {
    match std::fs::metadata(path) {
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => Some(None),
        // Not allowed to look is not the same as nothing there.
        Err(_) => None,
        Ok(meta) if !meta.is_file() || meta.len() > LARGEST => None,
        Ok(_) => std::fs::read_to_string(path).ok().map(Some),
    }
}

/// The changed regions between two texts, each with [`CONTEXT`] lines around it.
pub fn hunks(path: &str, was: &str, now: &str) -> Vec<Hunk> {
    if was == now {
        return Vec::new();
    }
    let diff = TextDiff::from_lines(was, now);
    diff.grouped_ops(CONTEXT)
        .iter()
        .map(|group| {
            let (mut before, mut after) = (String::new(), String::new());
            for op in group {
                for change in diff.iter_changes(op) {
                    match change.tag() {
                        similar::ChangeTag::Equal => {
                            before.push_str(change.value());
                            after.push_str(change.value());
                        }
                        similar::ChangeTag::Delete => before.push_str(change.value()),
                        similar::ChangeTag::Insert => after.push_str(change.value()),
                    }
                }
            }
            Hunk {
                path: path.to_string(),
                before,
                after,
                assumed: None,
                alternative: None,
            }
        })
        .collect()
}

/// The files a prediction names, as they are now.
fn look(paths: &[String]) -> Files {
    paths
        .iter()
        .filter_map(|path| Some((path.clone(), read(Path::new(path))?)))
        .collect()
}
