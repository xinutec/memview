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
//! `docs/execution-model.md`, "Two settings, one evaluator".
//!
//! Nothing here runs the command or writes a file it names.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use reader::predict::{Divergence, Files, Written, check, needs, predict};
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

/// A call whose files did not end up as predicted: the finding, in full.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub session: String,
    pub call: String,
    pub command: String,
    pub path: String,
    pub predicted: String,
    pub actual: Option<String>,
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

/// A prediction waiting for its call to finish.
struct Pending {
    session: String,
    command: String,
    written: Vec<Written>,
    since: std::time::Instant,
}

/// How long a prediction waits for its call to end. A backgrounded call in a
/// session the console does not run never reports its end here.
const HELD: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

#[derive(Default)]
pub struct Edits {
    root: PathBuf,
    pending: Mutex<HashMap<String, Pending>>,
}

impl Edits {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            pending: Mutex::default(),
        }
    }

    /// Predict what `command` will change, before it runs. `None` when it writes
    /// nothing the reader can follow.
    pub fn before(&self, session: &str, call: &str, command: &str, cwd: &str) -> Option<Edited> {
        let script = reader::syntax::parse(command).ok()?;
        let home = std::env::var("HOME").unwrap_or_default();
        let files: Files = needs(&script, cwd, &home)
            .into_iter()
            .filter_map(|path| Some((path.clone(), read(Path::new(&path))?)))
            .collect();
        let prediction = predict(&script, cwd, &home, &files);
        if prediction.written.is_empty() {
            return None;
        }
        // The text each file holds now, for the diff's first side.
        let hunks = prediction
            .written
            .iter()
            .flat_map(|written| {
                let now = read(Path::new(&written.path)).flatten().unwrap_or_default();
                hunks(&written.path, &now, &written.text)
            })
            .collect();
        let edited = Edited {
            call: call.to_string(),
            hunks,
        };
        let mut pending = self.pending.lock();
        pending.retain(|_, waiting| waiting.since.elapsed() < HELD);
        pending.insert(
            call.to_string(),
            Pending {
                session: session.to_string(),
                command: command.to_string(),
                written: prediction.written,
                since: std::time::Instant::now(),
            },
        );
        drop(pending);
        if let Err(why) = self.keep(&self.file(session), &edited) {
            tracing::warn!("could not keep the prediction of {call}: {why}");
        }
        Some(edited)
    }

    /// The call's hook says it is done. A call sent to the background has only
    /// started, so its check waits for [`Self::ended`]; any other is checked now.
    pub fn finished(&self, call: &str, response: &serde_json::Value) -> Option<(String, Diverged)> {
        if response
            .get("backgroundTaskId")
            .is_some_and(serde_json::Value::is_string)
        {
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
                self.pending.lock().remove(call);
                None
            }
        }
    }

    /// Whether the call left its files as predicted: the files that did not, each
    /// kept as a finding. `None` when there was no prediction for the call.
    fn check(&self, call: &str) -> Option<(String, Diverged)> {
        let pending = self.pending.lock().remove(call)?;
        let now: Files = pending
            .written
            .iter()
            .filter_map(|written| Some((written.path.clone(), read(Path::new(&written.path))?)))
            .collect();
        let diverged = check(&pending.written, &now);
        for divergence in &diverged {
            let finding = Finding {
                session: pending.session.clone(),
                call: call.to_string(),
                command: pending.command.clone(),
                path: divergence.path.clone(),
                predicted: divergence.predicted.clone(),
                actual: divergence.actual.clone(),
            };
            tracing::warn!("{call}: {} did not end up as predicted", divergence.path);
            if let Err(why) = self.keep(&self.root.join("findings.jsonl"), &finding) {
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

/// Every value one JSON line each in `file`, skipping any that do not read.
fn lines<T: serde::de::DeserializeOwned>(file: &Path) -> Vec<T> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// A file's text: `Some(None)` when it does not exist, `None` when it cannot be
/// read here — too large, not text, or not a regular file.
fn read(path: &Path) -> Option<Option<String>> {
    match std::fs::metadata(path) {
        Err(_) => Some(None),
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
            }
        })
        .collect()
}
