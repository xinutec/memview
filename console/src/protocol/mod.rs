//! The Claude Code stream-json protocol, read as far as the console needs it: one
//! line in, zero or more [`Event`]s out.
//!
//! Deliberately partial, and safe about it: every enum has an `Other` catch-all,
//! so a line the CLI grows tomorrow is ignored rather than fatal.
//!
//! The same content arrives twice — text as `stream_event` deltas and in the
//! complete `assistant` message; tool calls in `content_block_start` with partial
//! arguments and whole in the complete message. Text comes from the deltas, tool
//! calls from the completed message, nothing from both.
//!
//! Verified against CLI 2.1.220; `tests/fixtures/turn.jsonl` is a real capture.

mod background;
mod control;

use background::{finished, is_notification};

pub use background::{Called, Ended, Named, Running, called, running};
pub use control::{
    Annotation, Annotations, Answer, Answers, ModeReply, QUESTION_TOOL, Reply, SET_MODE, decision,
    get_usage, mode_reply, prompt, prompt_with_image, rename, set_mode, shown, usage_reply,
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// How much of what a tool returned is carried to the client. The head, because
/// failures are short and a message hiding at the end is the case almost never cut.
const RESULT_SNIPPET: usize = 2000;

/// An event and when it happened. Separate from [`Event`] because the time comes
/// from a different place: a live event is stamped when seen, a replayed one
/// carries the transcript's time. Flattened on the wire. `at` is optional because
/// a transcript line is entitled not to have one; milliseconds since the epoch.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Timed {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub at: Option<i64>,
    #[serde(flatten)]
    pub event: Event,
}

/// What the console tells its clients about a session: a small closed set a UI
/// can render, derived from a protocol that is neither.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Event {
    /// Where the console started watching: everything before it was read from the
    /// transcript, everything after it this console saw happen.
    Joined {
        earlier: usize,
        /// Where in the transcript the seed began, as a byte offset — the cursor for
        /// asking what came before. Zero means nothing older.
        from: u64,
        /// Whether the conversation was picked up by a new process, so a tool call above
        /// this line that never finished never will. The client marks those dead, and
        /// must not do it blindly: `Joined` is also emitted per reader at the end of a
        /// seed, where the last call in the page is the one running right now. `true`
        /// for [`crate::session::Session::resume`], `false` for an upgrade or a reader.
        #[serde(default)]
        restarted: bool,
    },
    /// The session is up, with what it was given to work with.
    Started {
        model: String,
        cwd: String,
        tools: usize,
    },
    /// A message this console has written to the session's stdin, before the CLI has
    /// read it. The gap to [`Event::Prompt`] is minutes when input arrives mid-turn,
    /// and with only the echo to go on a sent message looks like one that never
    /// arrived. This says *the runner has it*; `Prompt` says *the session has read it*.
    Accepted {
        text: String,
    },
    /// The prompt, echoed back by `--replay-user-messages`: the CLI's own
    /// acknowledgement that the message arrived.
    Prompt {
        text: String,
    },
    /// A slash command — `/compact`, `/context` — rather than something said to the
    /// model. A command has no read receipt: `--replay-user-messages` does not replay
    /// one, so a *waiting to be read* marker on it would never clear.
    /// Its own variant so that everything counting what a person said keeps counting
    /// what a person said.
    Command {
        text: String,
    },
    /// A picture sent to this session, by the name of the copy kept for it — enough
    /// to ask for it back at `/api/sessions/{id}/images/{name}`. The bytes are
    /// deliberately not here: a megabyte in an event stream of small messages.
    Shown {
        name: String,
    },
    Text {
        text: String,
    },
    /// How many tokens one request's prompt came to. Per message, not per turn: the
    /// result line's `usage` sums every request the turn made, and can exceed the
    /// window.
    Context {
        tokens: u64,
    },
    Tool {
        id: String,
        name: String,
        does: crate::call::Call,
    },
    /// A background task the harness has finished with, named by whichever of the
    /// two ids the notification gave. Its own event rather than a prompt, since the
    /// notification arrives filed as a *user* message, and the only end-of-work
    /// signal for a backgrounded command. Exactly one of the two is present;
    /// [`finished`] is the only thing that builds one.
    Background {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        tool: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        task: Option<String>,
        /// How it ended, when the notification says.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        status: Option<Ended>,
    },
    /// A tool call came back, with what it returned cut to [`RESULT_SNIPPET`]: a
    /// verdict alone says a `grep` succeeded, not what it found.
    ToolResult {
        id: String,
        ok: bool,
        /// What it returned, as text. Empty when it returned nothing that is text; an
        /// image result says so in words.
        detail: String,
        /// The full length in characters, present only when `detail` is a cut of it.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cut: Option<usize>,
        /// Whether it returned a picture, which a client can show from the call's path.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
        image: bool,
    },
    /// What a `Bash` call will change in the files it writes, as the reader
    /// predicts it. See [`crate::edits`].
    Edited {
        call: String,
        hunks: Vec<crate::edits::Hunk>,
    },
    /// A call whose files did not end up as predicted — a defect in the evaluator.
    Diverged {
        call: String,
        paths: Vec<String>,
    },
    /// One turn finished.
    Turn {
        cost_usd: f64,
        /// How big the context window is. Declared on the result line and nowhere else;
        /// how full it is comes from [`Event::Context`].
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        window: Option<u64>,
        turns: u32,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        stop: Option<String>,
    },
    /// A rate-limit window changed state, with its `utilization`. One window per
    /// event, so the windows are collected as they are seen.
    Limit {
        window: String,
        status: Allowance,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        resets_at: Option<i64>,
        /// How much of the window is spent, as a fraction. Optional in the CLI's
        /// schema, so optional here.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        utilization: Option<f64>,
    },
    /// The CLI's own progress reporting — "requesting", "tool_use", and so on.
    Busy {
        status: String,
    },
    /// The session wants to run something and is waiting to be told whether it may.
    Ask {
        /// The control request's id, which the answer must carry back.
        id: String,
        /// The call this is asking about — the `tool_use` id on the [`Event::Tool`] the
        /// CLI emitted a moment earlier. Without it one action draws two widgets.
        /// Optional because one of the CLI's three call sites omits it.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        call: Option<String>,
        tool: String,
        /// The CLI's own one-line rendering of the question, when it offers one.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        detail: Option<String>,
        does: crate::call::Call,
        /// The arguments as the CLI sent them, kept for the answer, which echoes them
        /// back with any answers written in. Never sent to a client, which reads
        /// [`Self::Ask::does`].
        #[serde(skip)]
        #[cfg_attr(feature = "ts", ts(skip))]
        input: serde_json::Value,
    },
    /// A question that has been answered, so a second client stops offering a
    /// decision already taken.
    Answered {
        id: String,
        allowed: bool,
        /// What was said, when the question was one a person answers. Carried here
        /// because an `ask` is a control request, not a transcript line, so a second
        /// screen or a reload has no other way to learn what was chosen.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        reply: Option<Reply>,
    },
    /// The conversation was compacted: everything above this was replaced by a
    /// summary. Only ever read from a transcript — the live stream does not announce
    /// it.
    Compacted,
    /// The session has stopped reading its stdin: messages were written, it is
    /// between turns, and it has not taken them. The console's own conclusion,
    /// announced once per episode — see [`crate::session::Session::deaf`].
    Deaf {
        /// How many messages are waiting in the pipe.
        unread: usize,
        /// How long it has been failing to take them, in seconds.
        seconds: u64,
    },
    /// The subprocess ended. Terminal: nothing follows it.
    Exited {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        code: Option<i32>,
    },
    /// Something the console itself could not do — a spawn failure, a line that was
    /// not JSON. An event rather than a log line, since a session that has silently
    /// stopped working is the failure mode worth spending one on.
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
    /// The CLI parking a message it has been handed but not read. The earliest
    /// record that background work has ended, by minutes: enqueued when the work
    /// finishes, dequeued when the running turn lets go. Kebab-case on the wire,
    /// unlike every other line here.
    #[serde(rename = "queue-operation")]
    QueueOperation {
        operation: String,
        #[serde(default)]
        content: Content,
    },
    /// A message the CLI parked and has now handed to the running turn — the only
    /// record of a message sent to a busy session; the CLI writes no `user` line
    /// for one. Only `queued_command`: `task_reminder` attachments would bury the
    /// conversation in machinery.
    Attachment {
        attachment: Attached,
    },
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

/// What an `attachment` line carries. Only the one kind that is a person speaking.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Attached {
    QueuedCommand {
        #[serde(default)]
        prompt: Content,
    },
    #[serde(other)]
    Other,
}

/// A question from the CLI. Only one subtype is answered here.
#[derive(Debug, Deserialize)]
#[serde(tag = "subtype", rename_all = "snake_case")]
enum Control {
    CanUseTool {
        tool_name: String,
        /// The call being asked about. See [`Event::Ask::call`].
        #[serde(default)]
        tool_use_id: Option<String>,
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
    /// Written where a compaction happened — see [`Event::Compacted`].
    CompactBoundary,
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
    // The wire names carry the `_delta` suffix; the variants do not.
    #[serde(rename = "text_delta")]
    Text { text: String },
    #[serde(other)]
    Other,
}

/// An Anthropic API message, of which only the content blocks matter here.
#[derive(Debug, Deserialize)]
struct Message {
    #[serde(default)]
    content: Content,
    /// What this one request carried. See [`Usage`].
    #[serde(default)]
    usage: Option<Usage>,
    /// Which model answered — read only to recognise the one that is not a model at
    /// all. See [`SYNTHETIC`].
    #[serde(default)]
    model: Option<String>,
}

/// The `model` a message carries when nothing generated it: how a slash command
/// answers, as one complete `assistant` message with no deltas before it. [`read`]
/// keeps only tool calls from a completed message, except from this one.
const SYNTHETIC: &str = "<synthetic>";

/// `content` is a list of blocks — except on user lines where it is a bare
/// string, and both shapes are in every transcript. Declared as `Vec<Block>`
/// alone, serde's `default` turns the string form into an empty list silently.
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
        /// Usually a bare string, occasionally a list of blocks. [`Content`] reads both.
        #[serde(default)]
        content: Content,
    },
    /// A picture a tool returned. Named rather than swallowed by `Other`: a result
    /// rendered as nothing reads as a tool that did nothing.
    Image,
    #[serde(other)]
    Other,
}

/// What a tool returned, as text a phone can hold, its true length when cut, and
/// whether it held a picture. Text blocks only, joined.
fn returned(content: Content) -> (String, Option<usize>, bool) {
    let blocks = content.blocks();
    let image = blocks.iter().any(|block| matches!(block, Block::Image));
    let text = blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::Text { text } => Some(text),
            Block::Image => Some("[an image]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let whole = text.chars().count();
    if whole <= RESULT_SNIPPET {
        return (text, None, image);
    }
    // By characters, not bytes: a cut in the middle of one leaves a string that is
    // not text.
    (
        text.chars().take(RESULT_SNIPPET).collect(),
        Some(whole),
        image,
    )
}

/// One transcript line's own record of when it happened. Read separately from
/// [`Line`]: the timestamp sits beside the `type` tag, on lines this reader
/// ignores as well as ones it does not.
#[derive(Debug, Deserialize)]
struct Recorded {
    #[serde(default)]
    timestamp: Option<String>,
}

pub fn recorded_at(line: &str) -> Option<i64> {
    let recorded: Recorded = serde_json::from_str(line).ok()?;
    let when = OffsetDateTime::parse(&recorded.timestamp?, &Rfc3339).ok()?;
    Some((when.unix_timestamp_nanos() / 1_000_000) as i64)
}

#[derive(Debug, Deserialize)]
struct Turn {
    #[serde(default)]
    total_cost_usd: f64,
    /// Keyed by model id. The context window is declared here and nowhere else.
    #[serde(default, rename = "modelUsage")]
    model_usage: std::collections::BTreeMap<String, ModelUsage>,
    #[serde(default)]
    num_turns: u32,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    stop_reason: Option<String>,
}

/// The tokens one request carried. Context used is all three added together, not
/// `input_tokens`: a cached prompt reports two tokens of input and half a million
/// of cache read.
#[derive(Debug, Deserialize)]
struct Usage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    cache_creation_input_tokens: u64,
    #[serde(default)]
    cache_read_input_tokens: u64,
}

impl Usage {
    fn prompt(&self) -> u64 {
        self.input_tokens + self.cache_creation_input_tokens + self.cache_read_input_tokens
    }
}

#[derive(Debug, Deserialize)]
struct ModelUsage {
    #[serde(default, rename = "contextWindow")]
    context_window: u64,
}

#[derive(Debug, Deserialize)]
struct Limit {
    #[serde(rename = "rateLimitType")]
    kind: String,
    #[serde(deserialize_with = "crate::named::by_name")]
    status: Allowance,
    #[serde(rename = "resetsAt")]
    resets_at: Option<i64>,
    /// A fraction, not a percentage.
    #[serde(default)]
    utilization: Option<f64>,
}

/// Read one output line into the events it carries. A line that is not JSON, or
/// JSON of an unknown shape, yields nothing — the CLI prints lines for its own
/// purposes, and a console that refused to run on meeting one would break every
/// release.
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
        // Not something the live stream says. Named so the match stays exhaustive and
        // the day it does arrive is a decision rather than a drop into `Other`.
        Line::System(System::CompactBoundary) => Vec::new(),
        Line::System(System::Other) => Vec::new(),
        // Also file-only. Read off the transcript — see [`crate::past::Appended::finished`].
        Line::QueueOperation { .. } => Vec::new(),
        Line::StreamEvent { event } => match event {
            Stream::ContentBlockDelta { delta } => match delta {
                Delta::Text { text } => vec![Event::Text { text }],
                Delta::Other => Vec::new(),
            },
            Stream::Other => Vec::new(),
        },
        // Text is taken from the deltas, so the completed message contributes only what
        // they cannot carry whole — unless nothing generated it, in which case there
        // were no deltas and this is the only copy. See [`SYNTHETIC`].
        Line::Assistant { message } => {
            // The context as it stood for this request, ahead of the blocks.
            let context = message.usage.as_ref().map(|usage| Event::Context {
                tokens: usage.prompt(),
            });
            let spoken = message.model.as_deref() == Some(SYNTHETIC);
            context
                .into_iter()
                .chain(
                    message
                        .content
                        .blocks()
                        .into_iter()
                        .filter_map(|block| match block {
                            Block::ToolUse { id, name, input } => Some(Event::Tool {
                                does: crate::call::Call::read(&name, &input),
                                id,
                                name,
                            }),
                            Block::Text { text } if spoken && !text.trim().is_empty() => {
                                Some(Event::Text { text })
                            }
                            _ => None,
                        }),
                )
                .collect()
        }
        // A replayed prompt: what this console sent, coming back — read by the same
        // function that reads one out of a transcript, so a picture appears when sent.
        Line::User { message } => from_user(message.content),
        // The delivery of a message sent while the session was working, read as the
        // prompt it is, so the echo that clears "waiting to be read" arrives.
        Line::Attachment {
            attachment: Attached::QueuedCommand { prompt },
        } => from_user(prompt),
        Line::Attachment { .. } => Vec::new(),
        Line::Result(turn) => vec![Event::Turn {
            // Whichever model answered; the largest is the honest answer if a turn ever
            // spanned two.
            window: turn
                .model_usage
                .values()
                .map(|model| model.context_window)
                .max()
                .filter(|window| *window > 0),
            cost_usd: turn.total_cost_usd,
            turns: turn.num_turns,
            duration_ms: turn.duration_ms,
            stop: turn.stop_reason,
        }],
        Line::RateLimitEvent { rate_limit_info } => vec![Event::Limit {
            window: rate_limit_info.kind,
            status: rate_limit_info.status,
            resets_at: rate_limit_info.resets_at,
            utilization: rate_limit_info.utilization,
        }],
        Line::ControlRequest {
            request_id,
            request:
                Control::CanUseTool {
                    tool_name,
                    tool_use_id,
                    input,
                    title,
                    description,
                },
        } => vec![Event::Ask {
            id: request_id,
            call: tool_use_id,
            does: crate::call::Call::read(&tool_name, &input),
            tool: tool_name,
            title,
            detail: description,
            input,
        }],
        Line::ControlRequest { .. } => Vec::new(),
        Line::Other => Vec::new(),
    }
}

/// Everything one user message says, live or read back off the disk — one
/// function, so the two readings cannot drift apart. Read across the blocks: a sent
/// picture is an image block and a text block, and neither says enough alone.
fn from_user(content: Content) -> Vec<Event> {
    let blocks = content.blocks();
    let carries = blocks.iter().any(|block| matches!(block, Block::Image));
    blocks
        .into_iter()
        .flat_map(|block| match block {
            Block::ToolResult {
                tool_use_id,
                is_error,
                content,
            } => {
                let (detail, cut, image) = returned(content);
                vec![Event::ToolResult {
                    id: tool_use_id,
                    ok: !is_error,
                    detail,
                    cut,
                    image,
                }]
            }
            Block::Text { text } if carries => {
                let (name, words) = shown(&text);
                // The picture first and the words after, in the order they were sent.
                name.map(|name| Event::Shown { name })
                    .into_iter()
                    .chain((!words.is_empty()).then_some(Event::Prompt { text: words }))
                    .collect()
            }
            Block::Text { text } => match finished(&text) {
                Some(event) => vec![event],
                None if is_notification(&text) => Vec::new(),
                None => match commanded(&text) {
                    Some(text) => vec![Event::Command { text }],
                    None if !is_plumbing(&text) => vec![Event::Prompt { text }],
                    None => Vec::new(),
                },
            },
            _ => Vec::new(),
        })
        .collect::<Vec<Event>>()
        .into_iter()
        .fold(Vec::new(), said_once)
}

/// Two blocks are two messages — unless they are the same words twice.
///
/// Both happen: a prompt reaching the CLI twice inside a millisecond is recorded
/// as one message of two identical blocks, and messages queued to a busy session
/// are handed over as one message with a block each — the common case. Joining the
/// blocks would break that: a joined echo matches neither waiting message, so both
/// stay *waiting to be read*.
///
/// The words are the only thing to tell them apart by. The one case this gets
/// wrong — the same words genuinely typed twice into a busy session — loses a
/// bubble, the milder failure.
fn said_once(mut said: Vec<Event>, event: Event) -> Vec<Event> {
    let again = matches!(
        (said.last(), &event),
        (Some(Event::Prompt { text: held }), Event::Prompt { text }) if held == text
    );
    if !again {
        said.push(event);
    }
    said
}

/// One line of a transcript on disk, which is not quite one line of the stream: a
/// transcript has no deltas, so the completed messages are the only source and
/// there is nothing to double.
pub fn read_recorded(line: &str) -> Vec<Event> {
    let Ok(parsed) = serde_json::from_str::<Line>(line) else {
        return Vec::new();
    };
    match parsed {
        Line::Assistant { message } => {
            // Same as the live reader: the context as it stood for this request, so a
            // resumed session knows its fullness before it finishes a turn.
            let context = message.usage.as_ref().map(|usage| Event::Context {
                tokens: usage.prompt(),
            });
            context
                .into_iter()
                .chain(
                    message
                        .content
                        .blocks()
                        .into_iter()
                        .filter_map(|block| match block {
                            Block::Text { text } => Some(Event::Text { text }),
                            Block::ToolUse { id, name, input } => Some(Event::Tool {
                                does: crate::call::Call::read(&name, &input),
                                id,
                                name,
                            }),
                            _ => None,
                        }),
                )
                .collect()
        }
        Line::User { message } => from_user(message.content),
        // The replay path needs this more than the live one: a message sent to a busy
        // session exists in the transcript only as this attachment.
        Line::Attachment {
            attachment: Attached::QueuedCommand { prompt },
        } => from_user(prompt),
        // Only what has just been handed over, and only a notification: `remove` is the
        // same message going the other way.
        Line::QueueOperation { operation, content } if operation == "enqueue" => content
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                Block::Text { text } => finished(&text),
                _ => None,
            })
            .collect(),
        Line::System(System::CompactBoundary) => vec![Event::Compacted],
        // Everything else a transcript carries belongs to the CLI, not a reader of
        // conversations.
        _ => Vec::new(),
    }
}

/// The account's own verdict on a rate-limit window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Allowance {
    Allowed,
    AllowedWarning,
    Rejected,
    Unknown(String),
}

impl crate::named::Named for Allowance {
    fn unknown(name: String) -> Self {
        Allowance::Unknown(name)
    }
}

/// What sits between two markers, when both are there and in that order.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim())
}

/// The slash command a message typed into the console is, if it is one — decided
/// on the way in, since a prompt is echoed and a command is not. A leading slash
/// and then a word running to whitespace: `/Users/pippijn/Code/…` is a path.
pub fn is_command(text: &str) -> bool {
    let Some(rest) = text.trim().strip_prefix('/') else {
        return false;
    };
    let word = rest.split_whitespace().next().unwrap_or_default();
    !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':'))
}

/// The command a recorded message is the expansion of, put back the way it was
/// typed: `<command-name>` and `<command-args>` together, since `/loop check` and
/// `/loop` are different messages. Either tag can open the message.
fn commanded(text: &str) -> Option<String> {
    let head = text.trim_start();
    if !head.starts_with("<command-name>") && !head.starts_with("<command-message>") {
        return None;
    }
    let name = between(head, "<command-name>", "</command-name>")?;
    let args = between(head, "<command-args>", "</command-args>").unwrap_or_default();
    Some(format!("{name} {args}").trim().to_string())
}

fn is_plumbing(text: &str) -> bool {
    const TAGS: [&str; 5] = [
        "<command-name>",
        "<local-command-stdout>",
        "<local-command-caveat>",
        "<system-reminder>",
        "<command-message>",
    ];
    let head = text.trim_start();
    TAGS.iter().any(|tag| head.starts_with(tag)) || is_picture_note(head)
}

/// The harness's note after every picture it hands the model:
/// `[Image: original 824x2300, displayed at 717x2000. Multiply coordinates by 1.15 to map to original image.]`
fn is_picture_note(text: &str) -> bool {
    text.starts_with("[Image: original ")
        && text.trim_end().ends_with(" to map to original image.]")
}
