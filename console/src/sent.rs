//! What became of each message a client gave an id, so a retry is never a second
//! message and a message is taken back only while the session has not read it.
//!
//! The phone keeps an outbox and retries with the same id until it hears back:
//! after a lost receipt it cannot know whether its message arrived, and only this
//! side can. The CLI does much of the work — it records a message under the
//! client's id as `uuid`, ignores a second with the same id, and takes one back
//! from its queue by id until a turn takes it in (`cancel_async_message`,
//! measured on 2.1.289) — so what is kept here is what the CLI cannot know: which
//! ids were taken back, and which this console already wrote.
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
    /// Taken back before the session read it: a retry of it is refused, so a slow
    /// one cannot land after the person has rewritten it.
    #[serde(alias = "cancelled")]
    Withdrawn,
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
    /// Held by this console for the turn's end.
    Held,
    /// Written to the CLI and not yet read: in its queue, where it can be taken back.
    Unread,
    /// The session has read it, at `at` (epoch milliseconds).
    Delivered { at: i64 },
    /// Neither here nor in the transcript.
    Nowhere,
}

/// Why a message, or taking one back, was refused.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Why {
    /// It was taken back; this is a late retry of it.
    Withdrawn,
    /// The session has read it.
    Read,
}

/// A refusal, with when the session read the message when that is known.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Refusal {
    pub why: Why,
    /// In epoch milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub at: Option<i64>,
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
pub enum Withdraw {
    /// Never reached the session: remember the id as taken back.
    Forget,
    /// Held here: release it, and remember the id as taken back.
    TakeBack,
    /// In the CLI's queue: ask it, which says whether a turn took it in first.
    AskCli,
    /// Already taken back.
    Done,
    Refuse(Refusal),
}

/// A message arriving under an id this console has `fate` for, standing `there`.
pub fn sending(fate: Option<Fate>, there: Where) -> Send {
    match (fate, there) {
        (Some(Fate::Withdrawn), _) => Send::Refuse(Refusal {
            why: Why::Withdrawn,
            at: None,
        }),
        // Written by a process that died before the CLI read it: the CLI ignores
        // the id if it did, so writing again is safe.
        (_, Where::Nowhere) => Send::Write,
        (_, Where::Held | Where::Unread | Where::Delivered { .. }) => Send::AlreadyHave,
    }
}

/// A request to take back a message this console has `fate` for, standing `there`.
pub fn withdrawing(fate: Option<Fate>, there: Where) -> Withdraw {
    match (fate, there) {
        (Some(Fate::Withdrawn), _) => Withdraw::Done,
        (_, Where::Delivered { at }) => Withdraw::Refuse(Refusal {
            why: Why::Read,
            at: Some(at),
        }),
        (_, Where::Unread) => Withdraw::AskCli,
        (_, Where::Held) => Withdraw::TakeBack,
        (_, Where::Nowhere) => Withdraw::Forget,
    }
}

/// Every id a client gave, kept on disk so a restart still refuses one taken back.
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
                        "{} could not be read, so no message taken back is remembered: {error}",
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
    // Read between turns, the message is a user line under its id; read mid-turn,
    // a `queued_command` attachment carrying it as `source_uuid`.
    let said = format!("\"uuid\":\"{id}\"");
    let delivered = format!("\"source_uuid\":\"{id}\"");
    String::from_utf8_lossy(&tail)
        .lines()
        .filter(|line| line.starts_with("{\""))
        .find(|line| {
            (line.contains(&said) && line.contains("\"type\":\"user\""))
                || (line.contains(&delivered) && line.contains("\"queued_command\""))
        })
        .and_then(crate::protocol::recorded_at)
}

/// Whether `id` can be a CLI message uuid: the CLI refuses anything else.
pub fn valid(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}
