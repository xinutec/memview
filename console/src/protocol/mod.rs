//! The Claude Code stream-json protocol, read as far as the console needs it: one
//! line in, zero or more [`Event`]s out.
//!
//! Partial on purpose: every enum has an `Other`, so a new line shape is ignored,
//! not fatal. Content arrives twice — as deltas and in the completed message — so
//! text is taken from the deltas and tool calls from the completed message.
//! `tests/fixtures/turn.jsonl` is a real capture.

mod background;
mod control;

use background::{finished, is_notification};

pub use background::{Called, Ended, Named, Running, called, running};
pub use control::{
    Annotation, Annotations, Answer, Answers, COMMANDS, Commands, ModeReply, ModelChoice, Picture,
    QUESTION_TOOL, Reply, SET_MODE, SlashCommand, UsageReply, commands_reply, decision, get_usage,
    initialize, mode_reply, prompt, prompt_with_images, rename, set_mode, shown, usage_reply,
};

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// How much of what a tool returned is carried to the client: the head.
const RESULT_SNIPPET: usize = 2000;

/// An event and when it happened, in epoch milliseconds: stamped when seen live,
/// the transcript's own time when replayed, absent when a line has none.
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

/// What the console tells its clients about a session: a small closed set.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Event {
    /// Where the console started watching: before it, read from the transcript.
    Joined {
        earlier: usize,
        /// The byte offset the seed began at — the cursor for older pages; zero is the start.
        from: u64,
        /// Whether a new process took the conversation over, so unfinished calls above
        /// never will finish. False for an upgrade or a reader's seed.
        #[serde(default)]
        restarted: bool,
    },
    /// The session is up, with what it was given to work with.
    Started {
        model: String,
        cwd: String,
        tools: usize,
    },
    /// A message written to stdin, not yet read: the runner has it. [`Event::Prompt`]
    /// says the session read it, which mid-turn can be minutes later.
    Accepted {
        text: String,
        /// The message's own id when a client gave it one: how a retry is told from
        /// a second message. See [`crate::sent`].
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        id: Option<String>,
    },
    /// The prompt, echoed back: the session has read it.
    Prompt {
        text: String,
        /// The message's own id when a client gave it one: how a retry is told from
        /// a second message. See [`crate::sent`].
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        id: Option<String>,
    },
    /// A slash command, not something said to the model. Never echoed, so never
    /// marked waiting to be read.
    Command {
        text: String,
    },
    /// One message with pictures: each by the name its copy is served under at
    /// `/api/sessions/{id}/images/{name}`, in the order sent, and what was said
    /// with them.
    Shown {
        names: Vec<String>,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        #[cfg_attr(feature = "ts", ts(as = "Option<String>", optional))]
        text: String,
        /// The message's own id when a client gave it one: how a retry is told from
        /// a second message. See [`crate::sent`].
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        id: Option<String>,
    },
    Text {
        text: String,
    },
    /// One request's prompt in tokens — per message; a turn's total can exceed the window.
    Context {
        tokens: u64,
    },
    Tool {
        id: String,
        name: String,
        does: crate::call::Call,
    },
    /// Background work the harness says has ended, by whichever id it gave — exactly
    /// one of the two. Built only by [`finished`].
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
    /// A tool call came back, with what it returned cut to [`RESULT_SNIPPET`].
    ToolResult {
        id: String,
        ok: bool,
        /// What it returned, as text.
        detail: String,
        /// The full length in characters, present only when `detail` is a cut of it.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        cut: Option<usize>,
        /// Whether it returned a picture.
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        #[cfg_attr(feature = "ts", ts(as = "Option<bool>", optional))]
        image: bool,
    },
    /// What a `Bash` call is predicted to change. See [`crate::edits`].
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
        /// The context window's size, declared only here.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        window: Option<u64>,
        turns: u32,
        duration_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        stop: Option<String>,
    },
    /// A rate-limit window changed state. One window per event.
    Limit {
        window: String,
        status: Allowance,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        resets_at: Option<i64>,
        /// How much of the window is spent, as a fraction.
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
        /// The `tool_use` id asked about, so one action draws one widget. One CLI path
        /// omits it.
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
        /// The arguments as sent, for the answer to echo back. Never sent to a client.
        #[serde(skip)]
        #[cfg_attr(feature = "ts", ts(skip))]
        input: serde_json::Value,
    },
    /// A question answered, so no other client offers it again.
    Answered {
        id: String,
        allowed: bool,
        /// What was said, for a question a person answers; no transcript records it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        reply: Option<Reply>,
    },
    /// The shared sign-in was refused: an assistant line whose `error` is
    /// `authentication_failed`. See [`crate::login`].
    SignedOut,
    /// The conversation was compacted. Only a transcript says so.
    Compacted,
    /// The session has stopped reading its stdin — the console's conclusion, once per
    /// episode. See [`crate::session::Session::deaf`].
    Deaf {
        /// How many messages are waiting in the pipe.
        unread: usize,
        /// For how long, in seconds.
        seconds: u64,
    },
    /// The subprocess ended. Terminal: nothing follows it.
    Exited {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        code: Option<i32>,
    },
    /// Something the console itself could not do, such as a spawn failure.
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
        /// The API error a synthetic message stands in for — `authentication_failed`,
        /// `rate_limit`, … — on the line itself, beside `message`.
        #[serde(default)]
        error: Option<String>,
    },
    User {
        message: Message,
        /// The message's id: the client's, when it gave one. See [`crate::sent`].
        #[serde(default)]
        uuid: Option<String>,
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

/// The `model` of a message nothing generated, such as a slash command's answer:
/// no deltas precede it, so [`read`] keeps its text.
const SYNTHETIC: &str = "<synthetic>";

/// The `error` an assistant line carries when the sign-in has lapsed.
const SIGNED_OUT: &str = "authentication_failed";

/// [`Event::SignedOut`], when a line's `error` says so. Last, after the words.
fn signed_out(error: Option<&str>) -> Option<Event> {
    (error == Some(SIGNED_OUT)).then_some(Event::SignedOut)
}

/// `content`: a list of blocks, or on user lines a bare string. Both must parse.
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

/// What a tool returned: its text blocks joined and cut, the full length if cut,
/// and whether it held a picture.
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

/// When a transcript line happened — on every line, read apart from [`Line`].
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

/// The tokens one request carried. The prompt is all three together: cached
/// prompts report almost nothing as `input_tokens`.
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

/// One output line, parsed once: an answer to something the console asked, or
/// what the session says.
#[derive(Debug)]
pub enum Output {
    Commands(Commands),
    Usage(UsageReply),
    Mode(ModeReply),
    Events(Vec<Event>),
}

/// What one output line is. Parsed once, where trying each reader in turn parsed
/// a long transcript line up to four times; only a `control_response` is offered
/// to the answer readers.
pub fn output(line: &str) -> Output {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
        return Output::Events(Vec::new());
    };
    if value.get("type").and_then(|it| it.as_str()) == Some("control_response") {
        if let Some(commands) = control::commands_answer(&value) {
            return Output::Commands(commands);
        }
        if let Some(usage) = control::usage_answer(&value) {
            return Output::Usage(usage);
        }
        if let Some(mode) = control::mode_answer(&value) {
            return Output::Mode(mode);
        }
    }
    // A line of no shape the console reads is nothing to say, as in `read`.
    let Ok(parsed) = serde_json::from_value::<Line>(value) else {
        return Output::Events(Vec::new());
    };
    Output::Events(events(parsed))
}

/// Read one output line into its events. Anything unknown yields nothing.
pub fn read(line: &str) -> Vec<Event> {
    let Ok(parsed) = serde_json::from_str::<Line>(line) else {
        return Vec::new();
    };
    events(parsed)
}

/// The events of one parsed output line.
fn events(parsed: Line) -> Vec<Event> {
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
        Line::Assistant { message, error } => assistant(message, error, Words::Unstreamed),
        // A replayed prompt: what this console sent, coming back — read by the same
        // function that reads one out of a transcript, so a picture appears when sent.
        Line::User { message, uuid } => from_user(message.content, uuid),
        // The delivery of a message sent while the session was working, read as the
        // prompt it is, so the echo that clears "waiting to be read" arrives.
        Line::Attachment {
            attachment: Attached::QueuedCommand { prompt },
        } => from_user(prompt, None),
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

/// Everything one user message says, live or on disk, read across its blocks: a
/// message with pictures is their image blocks and one text block.
fn from_user(content: Content, id: Option<String>) -> Vec<Event> {
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
                // One message, so one event: the pictures and the words together.
                let (names, text) = shown(&text);
                vec![Event::Shown {
                    names,
                    text,
                    id: id.clone(),
                }]
            }
            Block::Text { text } => match finished(&text) {
                Some(event) => vec![event],
                None if is_notification(&text) => Vec::new(),
                None => match commanded(&text) {
                    Some(text) => vec![Event::Command { text }],
                    None if !is_plumbing(&text) => vec![Event::Prompt {
                        text,
                        id: id.clone(),
                    }],
                    None => Vec::new(),
                },
            },
            _ => Vec::new(),
        })
        .collect::<Vec<Event>>()
        .into_iter()
        .fold(Vec::new(), said_once)
}

/// Two blocks are two messages — queued messages arrive one block each — unless
/// they are the same words twice, which a doubled delivery records. The same words
/// typed twice lose a bubble, the milder failure.
fn said_once(mut said: Vec<Event>, event: Event) -> Vec<Event> {
    let again = matches!(
        (said.last(), &event),
        (Some(Event::Prompt { text: held, .. }), Event::Prompt { text, .. }) if held == text
    );
    if !again {
        said.push(event);
    }
    said
}

/// Which of a completed assistant message's words to read.
enum Words {
    /// Only words no delta carried: a synthetic message's, and not blank. Live, the
    /// rest arrived as deltas already — see [`SYNTHETIC`].
    Unstreamed,
    /// All of them: a transcript has no deltas.
    All,
}

/// What one completed assistant message says, live or on disk alike: the context
/// as it stood for this request (so a resumed session knows its fullness before it
/// finishes a turn), its tool calls, the words [`Words`] picks, and a lapsed
/// sign-in last. One reading for both, so a fix cannot land in only one.
fn assistant(message: Message, error: Option<String>, words: Words) -> Vec<Event> {
    let context = message.usage.as_ref().map(|usage| Event::Context {
        tokens: usage.prompt(),
    });
    let unstreamed = message.model.as_deref() == Some(SYNTHETIC);
    let keep = |text: &str| match words {
        Words::All => true,
        Words::Unstreamed => unstreamed && !text.trim().is_empty(),
    };
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
                    Block::Text { text } if keep(&text) => Some(Event::Text { text }),
                    _ => None,
                }),
        )
        .chain(signed_out(error.as_deref()))
        .collect()
}

/// One line of a transcript on disk: no deltas, so text comes from the completed
/// messages.
pub fn read_recorded(line: &str) -> Vec<Event> {
    let Ok(parsed) = serde_json::from_str::<Line>(line) else {
        return Vec::new();
    };
    match parsed {
        Line::Assistant { message, error } => assistant(message, error, Words::All),
        Line::User { message, uuid } => from_user(message.content, uuid),
        // The replay path needs this more than the live one: a message sent to a busy
        // session exists in the transcript only as this attachment.
        Line::Attachment {
            attachment: Attached::QueuedCommand { prompt },
        } => from_user(prompt, None),
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

/// How hard the model works at an answer, as `/effort` sets it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Unknown(String),
}

impl crate::named::Named for Effort {
    fn unknown(name: String) -> Self {
        Effort::Unknown(name)
    }
}

/// Who answered, from a transcript's assistant line.
#[derive(Debug, Clone, PartialEq)]
pub struct Answered {
    /// When, in epoch milliseconds: a compaction copies older answers in after it.
    pub at: i64,
    pub model: String,
    /// Absent for a model with no effort setting, such as Haiku.
    pub effort: Option<Effort>,
}

/// The fields of an assistant line naming who answered. The effort is on the line,
/// not in the message, and only in the transcript: stdout never carries it.
#[derive(Debug, Deserialize)]
struct Answering {
    #[serde(rename = "type")]
    kind: String,
    message: Option<AnsweringMessage>,
    #[serde(default)]
    effort: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnsweringMessage {
    model: Option<String>,
}

/// [`Answered`], when `line` is a transcript's assistant line a model wrote.
pub fn answered(line: &str) -> Option<Answered> {
    let answer: Answering = serde_json::from_str(line).ok()?;
    let model = answer.message?.model?;
    if answer.kind != "assistant" || model == SYNTHETIC {
        return None;
    }
    Some(Answered {
        at: recorded_at(line)?,
        model,
        effort: answer.effort.as_deref().map(crate::named::named),
    })
}

/// What sits between two markers, when both are there and in that order.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim())
}

/// The slash command a typed message is, if any: a slash and a word, not a path.
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

/// The command a recorded message expands, as typed: name and arguments.
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
