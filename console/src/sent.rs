//! What became of each message a client gave an id, so a retry is never a second
//! message and a cancel never pretends.
//!
//! The phone keeps an outbox and retries with the same id until it hears back:
//! after a lost receipt it cannot know whether its message arrived, and only this
//! side can. The CLI does the rest of the work — it records a message under the
//! client's id as `uuid` and ignores a second one with the same id, live and after
//! `--resume` (measured on 2.1.289) — so what is kept here is what the CLI cannot
//! know: which ids were cancelled, and which this console already wrote.
//!
//! Where a message stands is asked of the session (held, unread, echoed) and, for
//! one an earlier process wrote, of the transcript, which records it under its id.

use std::collections::BTreeMap;
use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// How long an id is remembered. The phone gives up on a message it has not
/// managed to send in this time rather than retry an id forgotten here.
pub const KEEP_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// How far back a transcript is searched for an id: a message in doubt is a recent
/// one, and the file can be gigabytes.
const TAIL: u64 = 32 * 1024 * 1024;

/// What this console did with an id.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fate {
    /// Written to the session, or held for its turn's end.
    Written,
    /// Taken back before it reached the session: a retry of it is refused, so a slow
    /// one cannot land after the message that replaced it.
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Entry {
    session: String,
    /// When, in epoch milliseconds.
    at: i64,
    fate: Fate,
}

/// Where a message stands, as the session and its transcript see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Where {
    /// Held by this console for the turn's end: it can still be taken back.
    Held,
    /// Written to the CLI and not yet read: it will be, and cannot be taken back.
    Unread,
    /// The CLI has it, since `at` (epoch milliseconds).
    Delivered { at: i64 },
    /// Neither here nor in the transcript.
    Nowhere,
}

/// Why a message, or a cancel, was refused.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Why {
    /// It was cancelled; this is a late retry of it.
    Cancelled,
    /// The session already has it.
    Delivered,
    /// The session has been handed it and will read it: too late to take back.
    InFlight,
}

/// A refusal, with when the message got there when that is known.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Refusal {
    pub why: Why,
    /// In epoch milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub at: Option<i64>,
    /// About the message an edit replaces, not the edit: the original got there, so
    /// the edit was not sent.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
    pub original: bool,
}

impl Refusal {
    fn because(why: Why, at: Option<i64>) -> Self {
        Self {
            why,
            at,
            original: false,
        }
    }
}

/// What to do with a message arriving under an id.
#[derive(Debug, PartialEq)]
pub enum Send {
    Write,
    /// It is already on its way or there: answer as if written, write nothing.
    AlreadyHave,
    Refuse(Refusal),
}

/// What to do with a request to take a message back.
#[derive(Debug, PartialEq)]
pub enum Cancel {
    /// Never reached the session: remember the id as cancelled.
    Forget,
    /// Held here: release it, and remember the id as cancelled.
    TakeBack,
    /// Already cancelled.
    Done,
    Refuse(Refusal),
}

/// A message arriving under an id this console has `fate` for, standing `there`.
pub fn sending(fate: Option<Fate>, there: Where) -> Send {
    match (fate, there) {
        (Some(Fate::Cancelled), _) => Send::Refuse(Refusal::because(Why::Cancelled, None)),
        // Written by a process that died before the CLI read it: the CLI ignores
        // the id if it did, so writing again is safe.
        (_, Where::Nowhere) => Send::Write,
        (_, Where::Held | Where::Unread | Where::Delivered { .. }) => Send::AlreadyHave,
    }
}

/// A request to take back a message this console has `fate` for, standing `there`.
pub fn cancelling(fate: Option<Fate>, there: Where) -> Cancel {
    match (fate, there) {
        (Some(Fate::Cancelled), _) => Cancel::Done,
        (_, Where::Delivered { at }) => Cancel::Refuse(Refusal::because(Why::Delivered, Some(at))),
        (_, Where::Unread) => Cancel::Refuse(Refusal::because(Why::InFlight, None)),
        (_, Where::Held) => Cancel::TakeBack,
        (_, Where::Nowhere) => Cancel::Forget,
    }
}

/// Every id a client gave, kept on disk so a restart still refuses a cancelled one.
pub struct Sent {
    file: PathBuf,
    entries: Mutex<BTreeMap<String, Entry>>,
    /// Held across a decision and its write, so two requests for one id — a retry
    /// overtaking the first — cannot both find it unwritten.
    deciding: tokio::sync::Mutex<()>,
}

impl Sent {
    pub fn load(file: PathBuf) -> Self {
        let entries = match std::fs::read(&file) {
            Err(_) => BTreeMap::new(),
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(entries) => entries,
                Err(error) => {
                    tracing::error!(
                        "{} could not be read, so no cancelled message is remembered: {error}",
                        file.display()
                    );
                    BTreeMap::new()
                }
            },
        };
        Self {
            file,
            entries: Mutex::new(entries),
            deciding: tokio::sync::Mutex::new(()),
        }
    }

    /// Hold for one decision and the write it leads to.
    pub async fn deciding(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.deciding.lock().await
    }

    pub fn fate(&self, id: &str) -> Option<Fate> {
        self.entries.lock().get(id).map(|entry| entry.fate)
    }

    /// Record what became of `id`, and forget what is past [`KEEP_MS`].
    pub fn record(&self, id: &str, session: &str, fate: Fate, now: i64) {
        let entries = {
            let mut entries = self.entries.lock();
            entries.insert(
                id.to_string(),
                Entry {
                    session: session.to_string(),
                    at: now,
                    fate,
                },
            );
            entries.retain(|_, entry| now - entry.at < KEEP_MS);
            entries.clone()
        };
        let written = serde_json::to_vec(&entries)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(std::fs::write(&self.file, bytes)?));
        if let Err(error) = written {
            tracing::error!("{}: could not keep {id}: {error}", self.file.display());
        }
    }
}

/// When the transcript recorded the message `id`, if it did, in epoch
/// milliseconds: the CLI records a client's message under its id.
pub fn recorded_at(transcript: &Path, id: &str) -> Option<i64> {
    let mut file = std::fs::File::open(transcript).ok()?;
    let length = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(TAIL)))
        .ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let needle = format!("\"uuid\":\"{id}\"");
    String::from_utf8_lossy(&tail)
        .lines()
        .filter(|line| line.contains(&needle))
        .find(|line| line.starts_with("{\"") && line.contains("\"type\":\"user\""))
        .and_then(crate::protocol::recorded_at)
}

/// Whether `id` can be a CLI message uuid: the CLI refuses anything else.
pub fn valid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}
