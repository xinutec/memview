//! The Claude Code stream-json protocol, read as far as the console needs it: one
//! line in, zero or more [`Event`]s out.
//!
//! Deliberately partial, and safe about it: every enum has an `Other` catch-all,
//! so a line the CLI grows tomorrow is ignored rather than fatal.
//!
//! The same content arrives twice — text as `stream_event` deltas AND in the
//! complete `assistant` message; tool calls in `content_block_start` with partial
//! arguments AND whole in the complete message. Text comes from the deltas, tool
//! calls from the completed message, nothing from both.
//!
//! Verified against CLI 2.1.220; `tests/fixtures/turn.jsonl` is a real capture.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// How much of what a tool returned is carried to the client. The HEAD, because
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
        /// Whether the conversation was picked up by a NEW process, so a tool call above
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
    /// and with only the echo to go on a sent message looked like one that never
    /// arrived — so it got sent again. This says *the runner has it*; `Prompt` says
    /// *the session has read it*.
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
    /// How many tokens one request's prompt came to. Per MESSAGE, not per turn: the
    /// result line's `usage` sums every request the turn made — 1.6M against a 1M
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
        status: String,
    },
    /// A tool call came back, with what it returned cut to [`RESULT_SNIPPET`] — the
    /// verdict alone showed a `grep` had succeeded and not one word of what it found.
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
        status: String,
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
    /// A message the CLI parked and has now handed to the running turn — the ONLY
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

/// What one event says about work that was left running. A decision rather than
/// a mutation, so the four cases can be tested without a session.
#[derive(Debug, PartialEq, Eq)]
pub enum Running {
    /// A call was started and left going, named by its own id. Both names kept
    /// because the two endings speak different ones: a notification names the `tool`,
    /// a kill names the `task`. `task` is `None` for a detach whose id could not be
    /// read.
    Began { tool: String, task: Option<String> },
    /// The harness says one has finished, by whichever name the notification gave.
    Ended(Named),
    /// A task was stopped from here, named by its task id — the one way work ends
    /// that reports nothing afterwards.
    Killed(String),
    /// Nothing that was running still is — as far as anybody can tell.
    Gone,
    /// This event says nothing about background work.
    Quiet,
}

/// Which of a background task's two names an ending speaks. A type rather than a
/// convention: the count is keyed by the call, so an ending naming it is a
/// removal, while one naming the task is a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// The `tool_use` id of the call that started the work. What all but one
    /// kind of notification gives.
    Call(String),
    /// The task id the harness gave the work. What a kill speaks, and what a
    /// monitor's timeout speaks.
    Task(String),
}

/// Read one event for what it says about work left running.
///
/// `Joined` means "forget what you were counting": the replay is full of calls
/// backgrounded hours ago, and a re-seed replays that history and then joins.
/// `Started` is NOT that — the CLI emits an init line at the head of every turn.
///
/// A killed task is never reported finished, so the kill has to be read here:
/// 162 kills over this machine's transcripts, not one of them notified.
pub fn running(event: &Event) -> Running {
    match event {
        Event::ToolResult { id, detail, .. } => match (detached(detail), stopped(detail)) {
            (Some(task), _) => Running::Began {
                tool: id.clone(),
                task,
            },
            (None, Some(task)) => Running::Killed(task),
            (None, None) => Running::Quiet,
        },
        Event::Background { tool, task, .. } => match (tool, task) {
            (Some(call), _) => Running::Ended(Named::Call(call.clone())),
            (None, Some(task)) => Running::Ended(Named::Task(task.clone())),
            // Unreachable through `finished`, which builds no such event. Quiet rather than
            // a panic: an event naming nothing cannot bring a count down either way.
            (None, None) => Running::Quiet,
        },
        Event::Joined { .. } => Running::Gone,
        _ => Running::Quiet,
    }
}

/// What a tool says when it has left work running.
///
/// The call's ANSWER, not its arguments: only `Bash` accepts `run_in_background`,
/// and a foreground command outliving its timeout is moved to the background with
/// its input still saying `false`. Every tool that detaches says so in the first
/// words it returns. A reworded CLI UNDERCOUNTS, which is the safe direction.
///
/// The phrase must OPEN the result: a `contains` counted every result that quoted
/// one of these sentences — a grep, a `Read` of this module.
///
/// Returns the task id the harness gave the work; `Some(None)` is a detach whose
/// id could not be read.
fn detached(said: &str) -> Option<Option<String>> {
    /// One way a result opens when it has left work running: the words it starts
    /// with, and the marker its task id follows.
    struct Opening {
        opens: &'static str,
        before_id: &'static str,
    }
    const SAYS: [Opening; 5] = [
        // Bash, asked to detach.
        Opening {
            opens: "Command running in background with ID: ",
            before_id: "Command running in background with ID: ",
        },
        // Bash, moved to the background when it outran its timeout. The opening stops
        // short of the id because the timeout — a number that varies — sits between.
        Opening {
            opens: "Command did not complete within its",
            before_id: "and was moved to the background (ID: ",
        },
        // Agent, which runs in the background unless told otherwise. Its id is on the
        // following line, and the result asks that it never be repeated — so it is
        // matched on here and rendered nowhere.
        Opening {
            opens: "Async agent launched successfully",
            before_id: "\nagentId: ",
        },
        // Workflow, which has no foreground form.
        Opening {
            opens: "Workflow launched in background. Task ID: ",
            before_id: "Workflow launched in background. Task ID: ",
        },
        // Monitor, the tool the whole rule came from.
        Opening {
            opens: "Monitor started (task ",
            before_id: "Monitor started (task ",
        },
    ];
    let opening = SAYS
        .iter()
        .find(|opening| said.starts_with(opening.opens))?;
    Some(
        said.split_once(opening.before_id)
            .and_then(|(_, id)| id_at(id)),
    )
}

/// The task a kill has just ended, when this result is one. The stopping call
/// answers in JSON, so this is a prefix of the document: `{"message":"Successfully
/// stopped task: …`.
fn stopped(said: &str) -> Option<String> {
    id_at(said.strip_prefix(r#"{"message":"Successfully stopped task: "#)?)
}

/// The id starting here, which runs until whatever ends it — a full stop, a
/// comma, a bracket or a space.
fn id_at(rest: &str) -> Option<String> {
    let id: String = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    (!id.is_empty()).then_some(id)
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
    status: String,
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
            // The context as it stood for THIS request, ahead of the blocks.
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

/// The tool that asks the person a question rather than the machine a favour. It
/// arrives as an ordinary [`Event::Ask`], but approving it unchanged is not an
/// answer — see [`Answers`].
pub const QUESTION_TOOL: &str = "AskUserQuestion";

/// What was chosen: the question's own text, against the option label picked.
/// Labels, not indexes, since the CLI matches these against what it offered.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Answer {
    One(String),
    Many(Vec<String>),
}

/// Every answer to one [`QUESTION_TOOL`] call.
pub type Answers = std::collections::BTreeMap<String, Answer>;

/// Something said about one question, beside the option picked. Unlike
/// [`Reply::response`] this combines rather than overrides: the CLI reports
/// `"<question>"=(no option selected) notes: …` for a note with no choice.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Annotation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notes: Option<String>,
}

/// Notes against the questions of one call, by the question's own text.
pub type Annotations = std::collections::BTreeMap<String, Annotation>;

/// What a person said about a question: options picked, or words instead.
/// `response` and `answers` are alternatives — the CLI's result builder tests
/// `response` first and reports only that, so prose sent alongside
/// choices throws the choices away. The client is where that is made visible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Answers::is_empty")]
    pub answers: Answers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub response: Option<String>,
    /// Notes beside the choices — see [`Annotation`]. These travel WITH `answers`.
    #[serde(default, skip_serializing_if = "Annotations::is_empty")]
    pub annotations: Annotations,
}

impl Reply {
    /// Whether there is anything here to say. An empty reply would read to the
    /// session as "did not answer" — true, but better refused at the door.
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
            && self.response.as_deref().unwrap_or("").trim().is_empty()
            && !self
                .annotations
                .values()
                .any(|note| !note.notes.as_deref().unwrap_or("").trim().is_empty())
    }
}

/// The answer to a `can_use_tool` question, in the shape the CLI reads.
///
/// An allow must carry the arguments back: `updatedInput` lets a client edit what
/// it approves, and the console echoes the input unchanged. For [`QUESTION_TOOL`]
/// that edit is the whole point — its `call` reads `answers` out of its own
/// arguments, so approving unchanged yields *"The user did not answer the
/// questions."*
pub fn decision(
    id: &str,
    allowed: bool,
    input: &serde_json::Value,
    why: &str,
    reply: Option<&Reply>,
) -> String {
    let response = if allowed {
        serde_json::json!({"behavior": "allow", "updatedInput": answered(input, reply)})
    } else {
        serde_json::json!({"behavior": "deny", "message": why})
    };
    serde_json::json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": id, "response": response},
    })
    .to_string()
}

/// The approved input, with the answers written into it. Nothing invented when
/// there are none; a non-object input is left alone rather than replaced.
fn answered(input: &serde_json::Value, reply: Option<&Reply>) -> serde_json::Value {
    let Some(reply) = reply else {
        return input.clone();
    };
    let mut input = input.clone();
    if let Some(object) = input.as_object_mut() {
        if !reply.answers.is_empty() {
            object.insert("answers".to_string(), serde_json::json!(reply.answers));
        }
        // Only when there is something in it: writing the key at all says a choice was
        // overridden by nothing.
        if let Some(said) = reply.response.as_deref().filter(|s| !s.trim().is_empty()) {
            object.insert("response".to_string(), serde_json::json!(said));
        }
        // Blank notes are dropped: the CLI tests `notes` for truthiness, and an empty
        // one would make it report `(no option selected)` for a question nobody touched.
        let notes: Annotations = reply
            .annotations
            .iter()
            .filter(|(_, note)| !note.notes.as_deref().unwrap_or("").trim().is_empty())
            .map(|(question, note)| (question.clone(), note.clone()))
            .collect();
        if !notes.is_empty() {
            object.insert("annotations".to_string(), serde_json::json!(notes));
        }
    }
    input
}

/// Rename a conversation, over the CONTROL channel — the only way to rename a
/// session that is working. `/rename` is input, and input arriving mid-turn is
/// parked and released as a prompt the model reads as words. A control request
/// is out-of-band: `success` comes back at once and the transcript gains
/// `{"type":"custom-title",…}`. `title`, and it must be a string — the CLI's own
/// validation; a `-p` session supports the subtype.
pub fn rename(request_id: &str, title: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "rename_session", "title": title},
    })
    .to_string()
}

pub fn set_mode(request_id: &str, mode: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "set_permission_mode", "mode": mode},
    })
    .to_string()
}

/// What the request id of a mode change looks like, so its answer is found. One
/// per session: the answer to the older of two changes is of no interest.
pub const SET_MODE: &str = "set-mode-";

/// How a session answered a mode change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeReply {
    /// The mode the CLI says it is now in — NOT the one asked for.
    Now(String),
    /// The CLI refused, in its own words, which are better than any of ours.
    Refused(String),
}

/// A session's answer to a mode change, if that is what this line is. Read it,
/// or the screen claims a mode the CLI never entered. The two shapes:
///
/// ```text
/// {"subtype":"success","request_id":"…","response":{"mode":"acceptEdits"}}
/// {"subtype":"error","request_id":"…","error":"Cannot set permission mode to
///   bypassPermissions because the session was not launched with
///   --dangerously-skip-permissions"}
/// ```
///
/// The success carries the mode, so this confirms rather than assumes it.
/// Matched on the request id, unlike [`usage_reply`].
pub fn mode_reply(line: &str) -> Option<ModeReply> {
    let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
    if parsed.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let response = parsed.get("response")?;
    if !response.get("request_id")?.as_str()?.starts_with(SET_MODE) {
        return None;
    }
    match response.get("subtype")?.as_str()? {
        "success" => Some(ModeReply::Now(
            response.get("response")?.get("mode")?.as_str()?.to_string(),
        )),
        // A refusal with no words is still a refusal: the claim on screen comes down.
        "error" => Some(ModeReply::Refused(
            response
                .get("error")
                .and_then(|it| it.as_str())
                .unwrap_or("the session refused the change")
                .to_string(),
        )),
        _ => None,
    }
}

/// Ask the session what the account has spent. The only route to the routine
/// figures: `rate_limit_event` carries the percentage only when a threshold is
/// crossed, and the statusLine never runs for a headless session. `get_usage` is
/// the CLI's own answer — experimental, so nothing here insists on its shape.
pub fn get_usage(request_id: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "get_usage"},
    })
    .to_string()
}

/// The rate limits out of a `control_response`, if that is what this line is.
///
/// Read field by field: the CLI warns the shape may change, and a moved shape
/// yields no reading rather than a wrong one. A model's own allowance is not a
/// key beside the others — `model_scoped` is an ARRAY of `{display_name,
/// utilization, resets_at}`.
/// Not matched on a request id: any response carrying rate limits is an answer.
pub fn usage_reply(line: &str) -> Option<Vec<(String, f64, Option<i64>)>> {
    let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
    if parsed.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let limits = parsed
        .get("response")?
        .get("response")?
        .get("rate_limits")?
        .as_object()?;
    let mut found = Vec::new();
    for (window, seen) in limits {
        if window == MODEL_SCOPED {
            found.extend(scoped(seen));
            continue;
        }
        // `utilization` is a percentage here, where the stream event's is a fraction.
        // Both become a fraction.
        let Some(pct) = seen.get("utilization").and_then(|it| it.as_f64()) else {
            continue;
        };
        found.push((window.clone(), pct / 100.0, resets_at(seen)));
    }
    (!found.is_empty()).then_some(found)
}

/// Where the CLI files a window belonging to one model rather than to the plan.
const MODEL_SCOPED: &str = "model_scoped";

/// The per-model windows out of that array, named by the model. An entry with no
/// name is dropped: a bar labelled after a guess is worse than none.
fn scoped(value: &serde_json::Value) -> Vec<(String, f64, Option<i64>)> {
    let Some(entries) = value.as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get("display_name")?.as_str()?;
            let pct = entry.get("utilization")?.as_f64()?;
            Some((crate::usage::model_key(name), pct / 100.0, resets_at(entry)))
        })
        .collect()
}

/// When a window turns over, in epoch seconds, if it says.
fn resets_at(window: &serde_json::Value) -> Option<i64> {
    window
        .get("resets_at")
        .and_then(|it| it.as_str())
        .and_then(|it| OffsetDateTime::parse(it, &Rfc3339).ok())
        .map(|when| when.unix_timestamp())
}

/// One user message, in the shape the CLI reads on stdin. `\n` terminated by
/// the caller — the CLI reads a line at a time.
pub fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    })
    .to_string()
}

/// Everything one user message says, live or read back off the disk — one
/// function, since two copies drifted apart once. Read ACROSS the blocks: a sent
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
/// are handed over as ONE message with a block each. Joining every pair fixed the
/// first at the price of the second, which is the common one — and a joined echo
/// matched neither waiting message, so both stayed *waiting to be read*.
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

/// The phrase that ties a sent picture to the copy on disk. Written by
/// [`prompt_with_image`], read by [`shown`].
const ALSO_AT: &str = "the image is also at ";

/// The picture a user message carries, and the words with the note about it
/// taken out — the note is addressed to the session. Only the file name comes
/// back, never the directory.
pub fn shown(text: &str) -> (Option<String>, String) {
    let Some(open) = text.rfind(&format!("({ALSO_AT}")) else {
        return (None, text.to_string());
    };
    let from = open + ALSO_AT.len() + 1;
    let Some(shut) = text[from..].find(')') else {
        return (None, text.to_string());
    };
    let name = std::path::Path::new(&text[from..from + shut])
        .file_name()
        .and_then(|it| it.to_str())
        .map(String::from);
    let words = format!("{}{}", &text[..open], &text[from + shut + 1..]);
    (name, words.trim().to_string())
}

/// One user message carrying a picture and what was said about it. The CLI
/// forwards an `image` block on stdin as the API defines one.
/// The picture first, the words after: a question read before the thing it is
/// about is answered from the question alone.
///
/// The text also names where the console kept its copy, so a session can open the
/// file after the image is compacted away — and it is how [`shown`] finds the
/// picture again on replay: a recorded image block holds base64 and no name.
/// Both share [`ALSO_AT`].
pub fn prompt_with_image(
    text: &str,
    media_type: &str,
    base64: &str,
    kept: &std::path::Path,
) -> String {
    let said = match text.trim() {
        "" => format!("({ALSO_AT}{})", kept.display()),
        words => format!("{words}\n\n({ALSO_AT}{})", kept.display()),
    };
    serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [
                {"type": "image", "source": {"type": "base64", "media_type": media_type, "data": base64}},
                {"type": "text", "text": said},
            ],
        },
    })
    .to_string()
}

/// One line of a transcript ON DISK, which is not quite one line of the stream: a
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
        // session exists in the transcript ONLY as this attachment.
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

/// The tool call a task notification is about, and how it ended. Read for the
/// tool-use id, since that is what ties it to the call the client counted. Two
/// tags out of a known block; anything without both is not one of these.
fn finished(text: &str) -> Option<Event> {
    if !is_notification(text) {
        return None;
    }
    let status = between(text, "<status>", "</status>")
        .unwrap_or("done")
        .to_string();
    // The ordinary case by a wide margin: the count is keyed by the call.
    if let Some(call) = between(text, "<tool-use-id>", "</tool-use-id>") {
        return Some(Event::Background {
            tool: Some(call.to_string()),
            task: None,
            status,
        });
    }
    let task = between(text, "<task-id>", "</task-id>")?;
    ends_without_naming_the_call(text).then(|| Event::Background {
        tool: None,
        task: Some(task.to_string()),
        status,
    })
}

/// Whether a notification that names no call is nonetheless an ending. Most such
/// are a monitor's ordinary output; the endings are a monitor's timeout and the
/// harness giving up on an agent. A session's inherited tasks are a greeting.
/// Anchored to its own tag, never `contains`: a monitor quoting the timeout line
/// would end itself.
fn ends_without_naming_the_call(text: &str) -> bool {
    const TIMED_OUT: &str = "[Monitor timed out — re-arm if needed.]";
    const NO_RECORD: &str = "No completion record was found for background agent";
    between(text, "<event>", "</event>") == Some(TIMED_OUT)
        || between(text, "<summary>", "</summary>").is_some_and(|said| said.starts_with(NO_RECORD))
}

/// Whether this text is the harness reporting on a background task at all —
/// separate from parsing it, since one that cannot be parsed still must not
/// become a prompt. The block has to END the message: a bare `contains` ate the
/// compact summary. Both ends, because the harness sometimes prefixes a banner.
fn is_notification(text: &str) -> bool {
    text.contains("<task-notification>") && text.trim_end().ends_with("</task-notification>")
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
/// `/loop` are different messages. Both openings occur — 1,369 lead with the
/// name, 95 with the message.
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

/// A background call, named the way somebody looking at the strip would name it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Called {
    /// The tool, as the CLI names it: `Bash`, `Monitor`, `Agent`.
    pub tool: String,
    /// A short label for what this call is doing, when the input carries one.
    /// `None` when it does not — better unlabelled than a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub label: Option<String>,
    /// The harness's task id, which is what a kill names.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub task: Option<String>,
}

/// How long a label may be before it is cut: a `Bash` label falls back to the
/// command, which ran to several hundred characters.
const LABEL_MAX: usize = 60;

/// A call as the running strip names it: the tool, and [`crate::call::Call::label`]
/// flattened to one line and cut to [`LABEL_MAX`].
pub fn called(tool: &str, does: &crate::call::Call) -> Called {
    let label = does.label().map(|text| {
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if flat.chars().count() > LABEL_MAX {
            let cut: String = flat.chars().take(LABEL_MAX).collect();
            format!("{}…", cut.trim_end())
        } else {
            flat
        }
    });
    Called {
        tool: tool.to_owned(),
        label,
        task: None,
    }
}
