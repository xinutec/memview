//! The words for a message refused, and the transcript's memory of messages a
//! process before this one was handed. Where each message stands is kept with
//! its session — [`crate::session::Messages`].

use std::io::{Read as _, Seek as _, SeekFrom};
use std::path::Path;

use serde::Serialize;

/// How far back a transcript is searched for an id: a message in doubt is a recent
/// one, and the file can be gigabytes.
const TAIL: u64 = 32 * 1024 * 1024;

/// Why a message, or taking one back, was refused.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Why {
    /// It was taken back; this is a late copy of it, or a second request.
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
