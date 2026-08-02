//! The Claude Code stream-json protocol, read as far as the console needs it.
//!
//! The CLI speaks JSON lines in both directions under
//! `--input-format stream-json --output-format stream-json`. This module owns
//! the reading half: one line in, zero or more [`Event`]s out.
//!
//! **Deliberately partial, and structured so that partiality is safe.** Every
//! enum here has an `Other` catch-all, so a line the CLI grows tomorrow is
//! ignored rather than fatal. The alternative — modelling the protocol
//! exhaustively — would break the console on a CLI upgrade, and the CLI is not
//! versioned for us.
//!
//! **Where each fact is taken from matters**, because the same content arrives
//! twice. Assistant text is streamed as `stream_event` deltas *and* repeated in
//! the complete `assistant` message. Tool calls appear in `content_block_start`
//! with their arguments still arriving as partial JSON, *and* in the complete
//! message with the arguments whole. So: text and thinking come from the deltas,
//! because the point of them is that they arrive while they are being written;
//! tool calls come from the completed message, because half a JSON object is not
//! something to render. Nothing is taken from both, so nothing is doubled.
//!
//! Verified against CLI 2.1.220; `tests/fixtures/turn.jsonl` is a real capture.

use serde::{Deserialize, Serialize};

/// What the console tells its clients about a session.
///
/// This is the API's vocabulary, not the CLI's: a small closed set that a UI can
/// render, derived from a protocol that is neither small nor closed.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// The session is up, with what it was given to work with.
    Started {
        model: String,
        cwd: String,
        tools: usize,
    },
    /// The prompt, echoed back by `--replay-user-messages`. It is the CLI's own
    /// acknowledgement that the message arrived, which is worth showing: it is
    /// the difference between "sent" and "received" when the phone is on a
    /// train.
    Prompt {
        text: String,
    },
    Text {
        text: String,
    },
    Thinking {
        text: String,
    },
    Tool {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    ToolResult {
        id: String,
        ok: bool,
    },
    /// One turn finished.
    Turn {
        cost_usd: f64,
        turns: u32,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop: Option<String>,
    },
    /// A rate-limit window changed state. Note this is a *status*, not a
    /// percentage: the stream says allowed/warning/rejected and when the window
    /// resets, and the percentages only exist in the statusLine hook's input.
    Limit {
        window: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        resets_at: Option<i64>,
    },
    /// The CLI's own progress reporting — "requesting", "tool_use", and so on.
    Busy {
        status: String,
    },
    /// The subprocess ended. Terminal: nothing follows it.
    Exited {
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<i32>,
    },
    /// Something the console itself could not do — a spawn failure, a line that
    /// was not JSON. Surfaced rather than logged, because a session that has
    /// silently stopped working is the failure mode worth spending an event on.
    Trouble {
        detail: String,
    },
}

/// One line of the CLI's output.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Line {
    System(System),
    StreamEvent {
        event: Stream,
    },
    Assistant {
        message: Message,
    },
    User {
        message: Message,
    },
    Result(Turn),
    RateLimitEvent {
        rate_limit_info: Limit,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
enum System {
    Init(Init),
    Status {
        status: String,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Init {
    model: String,
    cwd: String,
    #[serde(default)]
    tools: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Stream {
    ContentBlockDelta {
        delta: Delta,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Delta {
    // The wire names carry the `_delta` suffix; the variants do not, because
    // `Delta::TextDelta` says it twice.
    #[serde(rename = "text_delta")]
    Text { text: String },
    #[serde(rename = "thinking_delta")]
    Thinking { thinking: String },
    #[serde(other)]
    Other,
}

/// An Anthropic API message, of which only the content blocks matter here.
#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    content: Vec<Block>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: serde_json::Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
struct Turn {
    #[serde(default)]
    total_cost_usd: f64,
    #[serde(default)]
    num_turns: u32,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Limit {
    #[serde(rename = "rateLimitType")]
    kind: String,
    status: String,
    #[serde(rename = "resetsAt")]
    resets_at: Option<i64>,
}

/// Read one output line into the events it carries.
///
/// A line that is not JSON, or is JSON of a shape this does not know, yields
/// nothing. That is the intended behaviour and not a swallowed error: the CLI
/// prints lines for its own purposes, and a console that refused to run when it
/// met one would be broken by every release.
pub fn read(line: &str) -> Vec<Event> {
    let Ok(parsed) = serde_json::from_str::<Line>(line) else {
        return Vec::new();
    };
    match parsed {
        Line::System(System::Init(init)) => vec![Event::Started {
            model: init.model,
            cwd: init.cwd,
            tools: init.tools.len(),
        }],
        Line::System(System::Status { status }) => vec![Event::Busy { status }],
        Line::System(System::Other) => Vec::new(),
        Line::StreamEvent { event } => match event {
            Stream::ContentBlockDelta { delta } => match delta {
                Delta::Text { text } => vec![Event::Text { text }],
                Delta::Thinking { thinking } => vec![Event::Thinking { text: thinking }],
                Delta::Other => Vec::new(),
            },
            Stream::Other => Vec::new(),
        },
        // Text is taken from the deltas, so the completed message contributes
        // only what the deltas cannot carry whole.
        Line::Assistant { message } => message
            .content
            .into_iter()
            .filter_map(|block| match block {
                Block::ToolUse { id, name, input } => Some(Event::Tool { id, name, input }),
                _ => None,
            })
            .collect(),
        Line::User { message } => message
            .content
            .into_iter()
            .filter_map(|block| match block {
                Block::ToolResult {
                    tool_use_id,
                    is_error,
                } => Some(Event::ToolResult {
                    id: tool_use_id,
                    ok: !is_error,
                }),
                // A replayed prompt: the text this console sent, coming back.
                Block::Text { text } => Some(Event::Prompt { text }),
                _ => None,
            })
            .collect(),
        Line::Result(turn) => vec![Event::Turn {
            cost_usd: turn.total_cost_usd,
            turns: turn.num_turns,
            duration_ms: turn.duration_ms,
            stop: turn.stop_reason,
        }],
        Line::RateLimitEvent { rate_limit_info } => vec![Event::Limit {
            window: rate_limit_info.kind,
            status: rate_limit_info.status,
            resets_at: rate_limit_info.resets_at,
        }],
        Line::Other => Vec::new(),
    }
}

/// One user message, in the shape the CLI reads on stdin.
///
/// The whole input protocol the console needs: a text message, as an API user
/// message. `\n` terminated by the caller — the CLI reads a line at a time.
pub fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    })
    .to_string()
}
