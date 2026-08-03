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
    /// Where the console started watching. Everything before it was read from
    /// the transcript on disk; everything after it, this console saw happen.
    /// Worth marking rather than blending: the two have different warranties —
    /// one is what the file says, the other is what we watched.
    Joined {
        earlier: usize,
    },
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
    /// The session wants to run something and is waiting to be told whether it
    /// may. Nothing else happens in that session until it is answered.
    Ask {
        /// The control request's id, which the answer must carry back.
        id: String,
        tool: String,
        /// The CLI's own one-line rendering of the question, when it offers one
        /// — better than anything reconstructed from the arguments.
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        input: serde_json::Value,
    },
    /// A question that has been answered, by whom and how. Sent so that a second
    /// client watching the same session stops offering a decision that has
    /// already been taken.
    Answered {
        id: String,
        allowed: bool,
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
    ControlRequest {
        request_id: String,
        request: Control,
    },
    #[serde(other)]
    Other,
}

/// A question from the CLI. Only one subtype is answered here; the rest are for
/// clients that offer more than this one does.
#[derive(Debug, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
enum Control {
    CanUseTool {
        tool_name: String,
        #[serde(default)]
        input: serde_json::Value,
        #[serde(default)]
        title: Option<String>,
        #[serde(default)]
        description: Option<String>,
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
    content: Content,
}

/// ⚠ `content` is a list of blocks — except on the user lines where it is a bare
/// string. Both shapes are in every transcript on this machine. Declared as
/// `Vec<Block>` alone, the string form does not fail: serde's `default` makes it
/// an empty list, so those turns vanish silently, which reads as a conversation
/// with gaps in it rather than as a parse error.
#[derive(Debug, Deserialize, Default)]
#[serde(untagged)]
enum Content {
    Blocks(Vec<Block>),
    Text(String),
    #[default]
    #[serde(skip)]
    Missing,
}

impl Content {
    fn blocks(self) -> Vec<Block> {
        match self {
            Content::Blocks(blocks) => blocks,
            Content::Text(text) => vec![Block::Text { text }],
            Content::Missing => Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Block {
    Text {
        text: String,
    },
    /// Only a recorded transcript carries this whole; live, thinking arrives as
    /// deltas. See [`read_recorded`].
    Thinking {
        thinking: String,
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
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                Block::ToolUse { id, name, input } => Some(Event::Tool { id, name, input }),
                _ => None,
            })
            .collect(),
        Line::User { message } => message
            .content
            .blocks()
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
        Line::ControlRequest {
            request_id,
            request:
                Control::CanUseTool {
                    tool_name,
                    input,
                    title,
                    description,
                },
        } => vec![Event::Ask {
            id: request_id,
            tool: tool_name,
            title,
            detail: description,
            input,
        }],
        Line::ControlRequest { .. } => Vec::new(),
        Line::Other => Vec::new(),
    }
}

/// The answer to a `can_use_tool` question, in the shape the CLI reads.
///
/// **An allow must carry the arguments back.** The protocol lets a client edit
/// what it is approving — that is what `updatedInput` is for — and the console
/// approves what was asked, so it echoes the input unchanged rather than
/// omitting it. A deny carries a message, which the session sees as the reason
/// and can act on.
pub fn decision(id: &str, allowed: bool, input: &serde_json::Value, why: &str) -> String {
    let response = if allowed {
        serde_json::json!({"behavior": "allow", "updatedInput": input})
    } else {
        serde_json::json!({"behavior": "deny", "message": why})
    };
    serde_json::json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": id, "response": response},
    })
    .to_string()
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

/// One line of a transcript *on disk*, which is not quite one line of the stream.
///
/// ⚠ **A transcript has no deltas.** The live reader takes assistant text from
/// `stream_event` deltas and deliberately drops it from the completed message, so
/// that a sentence is not shown twice ([`read`]). A file recorded the completed
/// messages and nothing else — so replaying it through `read` yields a
/// conversation of tool calls with silence in between, which looks like an
/// unfinished feature rather than like the wrong reader.
///
/// So this is the same vocabulary read the other way round: completed messages
/// are the only source, and there is nothing to double.
pub fn read_recorded(line: &str) -> Vec<Event> {
    let Ok(parsed) = serde_json::from_str::<Line>(line) else {
        return Vec::new();
    };
    match parsed {
        Line::Assistant { message } => message
            .content
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                Block::Text { text } => Some(Event::Text { text }),
                Block::Thinking { thinking } => Some(Event::Thinking { text: thinking }),
                Block::ToolUse { id, name, input } => Some(Event::Tool { id, name, input }),
                _ => None,
            })
            .collect(),
        Line::User { message } => message
            .content
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                Block::ToolResult {
                    tool_use_id,
                    is_error,
                } => Some(Event::ToolResult {
                    id: tool_use_id,
                    ok: !is_error,
                }),
                Block::Text { text } if !is_plumbing(&text) => Some(Event::Prompt { text }),
                _ => None,
            })
            .collect(),
        // Everything else a transcript carries — the bridge lines, the file
        // snapshots, the summaries — belongs to the CLI and not to a reader of
        // conversations.
        _ => Vec::new(),
    }
}

/// Whether a recorded user turn is the CLI talking to itself.
///
/// A transcript files several things as user messages that nobody typed: the
/// echo of a slash command and its output, the caveat attached to local command
/// results, the reminders injected around a turn. Replayed as prompts they
/// outnumber the real ones and are indistinguishable from them on screen —
/// `/exit`, `Goodbye!` and `<system-reminder>` reading as things the person said.
///
/// Recognised by their opening tag, which is how they are delimited, rather than
/// by matching their contents. Live sessions are left alone: the console sends
/// its own prompts and echoes only what it sent.
fn is_plumbing(text: &str) -> bool {
    const TAGS: [&str; 5] = [
        "<command-name>",
        "<local-command-stdout>",
        "<local-command-caveat>",
        "<system-reminder>",
        "<command-message>",
    ];
    let head = text.trim_start();
    TAGS.iter().any(|tag| head.starts_with(tag))
}
