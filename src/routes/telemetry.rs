//! Client activity trace: what the browser sees and the API does not.
//!
//! Taps and navigations the API never hears about, folded into the same log as
//! its requests. Logs, not analytics: nothing is stored.

use axum::Json;
use axum::http::StatusCode;
use serde::Deserialize;

use crate::access::Owner;

/// One thing that happened in the client.
///
/// `kind` is `nav` for a route change, where `label` is absent, or `tap` for a
/// control, where `label` is its visible text, verbatim.
#[derive(Debug, Deserialize)]
pub struct TelemetryEvent {
    pub kind: String,
    pub path: String,
    #[serde(default)]
    pub label: Option<String>,
    /// The client's clock, in epoch milliseconds, which orders a batch.
    pub at: i64,
}

/// Most events accepted from one POST, against a log flood.
const MAX_EVENTS: usize = 100;

/// Longest label kept, in characters, so no glyph is split.
const MAX_LABEL: usize = 160;

/// Format characters that hide or reorder text: zero-width characters, and bidi
/// overrides, which can make a log line display something other than it says. A
/// deny-list, not all of category Cf.
fn is_deceptive_format(c: char) -> bool {
    matches!(c,
        '\u{00ad}'
        | '\u{200b}'..='\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{2069}'
        | '\u{feff}'
    )
}

/// Flatten a client-supplied label to one harmless log field — the endpoint's
/// security boundary: a newline would forge whole log lines. Control characters
/// and line separators become spaces, and the result is capped.
pub fn one_line(label: &str, max: usize) -> String {
    let unbroken: String = label
        .chars()
        .map(|c| {
            if c.is_control() || is_deceptive_format(c) {
                ' '
            } else {
                c
            }
        })
        .collect();
    unbroken
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// `POST /api/telemetry` — fold the client's events into the log. Always 204.
pub async fn record(Owner(user): Owner, Json(events): Json<Vec<TelemetryEvent>>) -> StatusCode {
    for e in events.into_iter().take(MAX_EVENTS) {
        let label = one_line(&e.label.unwrap_or_default(), MAX_LABEL);
        tracing::info!(
            user = %user.user_id,
            kind = %e.kind,
            path = %e.path,
            label = %label,
            at = e.at,
            "client-event"
        );
    }
    StatusCode::NO_CONTENT
}
