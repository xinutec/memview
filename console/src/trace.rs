//! Client activity trace: what the browser sees and the API does not.
//!
//! The API log already records that a session was told something. It cannot
//! record that a person meant to send it, found the control, waited, or gave up
//! — and on a four-inch screen that is most of what goes wrong. The events fold
//! into the same log stream as the requests, so a session reads as one timeline.
//!
//! **No storage.** These are logs, not data: the endpoint moves the events into
//! the backend log and forgets them.
//!
//! The same shape as memview's `routes/telemetry.rs`, and separate on purpose:
//! the console links nothing from the viewer.

use axum::Json;
use axum::http::StatusCode;
use serde::Deserialize;

/// Most a client may send in one flush. A trace that can flood the log is a way
/// to hide something in it.
const MAX_EVENTS: usize = 200;
const MAX_LABEL: usize = 200;

#[derive(Debug, Deserialize)]
pub struct Trace {
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub at: Option<i64>,
}

/// Flatten a client-supplied label to one line.
///
/// A newline in a label would let a caller forge log entries that appear to come
/// from somewhere else, and a log that can be forged stops being evidence.
/// Control characters become spaces, runs of whitespace collapse, and the result
/// is capped — `is_control` misses U+2028/U+2029, which `split_whitespace`
/// catches, so the two passes together cover both.
pub fn one_line(label: &str, max: usize) -> String {
    label
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// `POST /api/telemetry` — fold the client's events into the log stream.
///
/// Always 204. Best-effort by design: the client neither reads the response nor
/// retries, because a trace that interferes with the app it observes is worse
/// than no trace.
pub async fn record(Json(events): Json<Vec<Trace>>) -> StatusCode {
    for event in events.into_iter().take(MAX_EVENTS) {
        tracing::info!(
            kind = %one_line(&event.kind, 40),
            path = %one_line(&event.path, MAX_LABEL),
            label = %one_line(&event.label.unwrap_or_default(), MAX_LABEL),
            at = event.at,
            "client-event"
        );
    }
    StatusCode::NO_CONTENT
}
