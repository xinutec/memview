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
//! message with the arguments whole. So: text comes from the deltas,
//! because the point of them is that they arrive while they are being written;
//! tool calls come from the completed message, because half a JSON object is not
//! something to render. Nothing is taken from both, so nothing is doubled.
//!
//! Verified against CLI 2.1.220; `tests/fixtures/turn.jsonl` is a real capture.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// How much of what a tool returned is carried to the client.
///
/// Chosen against the transcripts on this machine rather than guessed: over
/// 151,000 recorded results, the median is 212 characters and 89% are shorter
/// than this. The tail is long — the largest is 76,000 — which is the whole
/// reason for a cap, but the common case is a handful of lines and arrives whole.
///
/// Failures are shorter still (median 127, 96% whole at this length), which is
/// what makes taking the **head** rather than the tail the right cut: the case
/// where a message hides at the end is the case that is almost never cut at all.
const RESULT_SNIPPET: usize = 2000;

/// An event and when it happened.
///
/// Separate from [`Event`] rather than a field on each variant, because the time
/// is not part of what happened — it is what the console can say about it, and it
/// comes from a different place depending on which. A live event is stamped when
/// the console sees it; a replayed one carries the time the transcript recorded,
/// which may be months ago. Flattened on the wire, so a client sees one object.
///
/// `at` is optional because a transcript line is entitled not to have one, and a
/// conversation is still worth reading when it does not say when it happened. It
/// is milliseconds since the epoch, like everything else numeric here.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Timed {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<i64>,
    #[serde(flatten)]
    pub event: Event,
}

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
        /// Where in the transcript the seed began, as a byte offset — the cursor
        /// for asking what came before it. Zero means the seed reached the start
        /// of the file and there is nothing older.
        from: u64,
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
    /// How many tokens one request's prompt came to — the context as it stood
    /// when that request was made.
    ///
    /// ⚠ **Per message, not per turn.** The result line carries a `usage` too,
    /// and it is the *sum over every request the turn made* — a turn of 23
    /// requests reported 1.6M against a 1M window, which is not a fullness at
    /// all. Measured, after shipping exactly that.
    Context {
        tokens: u64,
    },
    Tool {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// A background task the harness has finished with, named by the tool call
    /// that started it.
    ///
    /// Its own event rather than a prompt: the notification arrives filed as a
    /// *user* message, so without this it renders as something the person
    /// typed. It is also the only end-of-work signal for a backgrounded
    /// command — the tool call that started one returns at once, so nothing
    /// else says the work is over.
    Background {
        tool: String,
        status: String,
    },
    /// A tool call came back.
    ///
    /// ⚠ **`ok` alone is not an answer.** This carried the verdict and nothing
    /// else, so reading a session back showed that a `grep` had run and succeeded
    /// and not one word of what it found — which is usually the thing somebody
    /// scrolled back to see. What it returned is here, cut to [`RESULT_SNIPPET`].
    ToolResult {
        id: String,
        ok: bool,
        /// What it returned, as text. Empty when it returned nothing, or nothing
        /// that is text — an image result says so in words instead.
        #[serde(skip_serializing_if = "String::is_empty")]
        detail: String,
        /// The full length in characters, present only when `detail` is a cut of
        /// it. A snippet that does not admit to being one is a lie about what the
        /// tool said.
        #[serde(skip_serializing_if = "Option::is_none")]
        cut: Option<usize>,
    },
    /// One turn finished.
    Turn {
        cost_usd: f64,
        /// How big the context window is. Declared on the result line and
        /// nowhere else; how *full* it is comes from [`Event::Context`].
        #[serde(skip_serializing_if = "Option::is_none")]
        window: Option<u64>,
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
        /// What was said, when the question was one a person answers.
        ///
        /// **Carried here because this is the only place that knows it for
        /// everybody.** The client that tapped has it in hand; a second screen
        /// watching the same session, and the one that tapped after a reload, do
        /// not — and an `ask` is a control request rather than a transcript
        /// line, so [`crate::past`] cannot hand it back either. Sending it with
        /// the verdict is what lets an answered card say what was chosen instead
        /// of only that something was.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<Reply>,
    },
    /// The conversation was compacted: everything above this was replaced by a
    /// summary, and the session carried on with the shorter history.
    ///
    /// Only ever read from a transcript. The live stream does not announce a
    /// compaction — the CLI writes the boundary to the file and says nothing on
    /// stdout — so a session watched from the start will not see one happen,
    /// while a session seeded from disk will find every one that already had.
    Compacted,
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
    /// Written where a compaction happened. The CLI's own name for it — see
    /// [`Event::Compacted`] for why it is only ever seen in a file.
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
    // The wire names carry the `_delta` suffix; the variants do not, because
    // `Delta::TextDelta` says it twice.
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
        /// ⚠ Usually a bare string, sometimes a list of blocks — 98.4% against
        /// 1.6% across the transcripts on this machine. [`Content`] already reads
        /// both shapes, because the same split exists on user messages.
        #[serde(default)]
        content: Content,
    },
    /// A picture a tool returned — a screenshot, a page of a PDF. Named rather
    /// than swallowed by `Other`: a result rendered as nothing at all reads as a
    /// tool that did nothing, and there is no way to tell the two apart on screen.
    Image,
    #[serde(other)]
    Other,
}

/// What a tool returned, as text a phone can hold, and its true length when cut.
///
/// Text blocks only, joined. A tool result is not a document — it is what the
/// session was told — so the shape it had on the wire is not worth preserving.
fn returned(content: Content) -> (String, Option<usize>) {
    let text = content
        .blocks()
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
        return (text, None);
    }
    // By characters, not bytes: a cut in the middle of one leaves a string that
    // is not text, and every transcript here has some.
    (text.chars().take(RESULT_SNIPPET).collect(), Some(whole))
}

/// One transcript line's own record of when it happened.
///
/// Read separately from [`Line`] rather than added to each of its variants: the
/// timestamp sits beside the `type` tag rather than inside the message, and it is
/// on lines this reader ignores as well as ones it does not.
#[derive(Debug, Deserialize)]
struct Recorded {
    #[serde(default)]
    timestamp: Option<String>,
}

/// When a transcript line says it happened, in milliseconds since the epoch.
///
/// RFC 3339, which is what Claude Code writes (`2026-08-03T10:04:00.000Z`).
/// Parsed here rather than passed on as a string so that a line whose stamp we
/// cannot read becomes "no time" once, on the way in, instead of an
/// `Invalid Date` on a phone.
pub fn recorded_at(line: &str) -> Option<i64> {
    let recorded: Recorded = serde_json::from_str(line).ok()?;
    let when = OffsetDateTime::parse(&recorded.timestamp?, &Rfc3339).ok()?;
    Some((when.unix_timestamp_nanos() / 1_000_000) as i64)
}

#[derive(Debug, Deserialize)]
struct Turn {
    #[serde(default)]
    total_cost_usd: f64,
    /// Keyed by model id. The context window is declared here and nowhere else
    /// — not in the transcript, not on any other line.
    #[serde(default, rename = "modelUsage")]
    model_usage: std::collections::BTreeMap<String, ModelUsage>,
    #[serde(default)]
    num_turns: u32,
    #[serde(default)]
    duration_ms: u64,
    #[serde(default)]
    stop_reason: Option<String>,
}

/// The tokens one request carried.
///
/// ⚠ **Context used is all three added together**, not `input_tokens`. A cached
/// prompt reports two tokens of input and half a million of cache read — taking
/// the first would say a session near its limit was empty.
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
        // Not something the live stream says — the CLI writes the boundary to
        // its transcript and announces nothing on stdout. Named here so the
        // match stays exhaustive and the day it *does* arrive is a decision
        // rather than a silent drop into `Other`.
        Line::System(System::CompactBoundary) => Vec::new(),
        Line::System(System::Other) => Vec::new(),
        Line::StreamEvent { event } => match event {
            Stream::ContentBlockDelta { delta } => match delta {
                Delta::Text { text } => vec![Event::Text { text }],
                Delta::Other => Vec::new(),
            },
            Stream::Other => Vec::new(),
        },
        // Text is taken from the deltas, so the completed message contributes
        // only what the deltas cannot carry whole.
        Line::Assistant { message } => {
            // The context as it stood for THIS request, ahead of the blocks, so
            // a reader sees the fullness that produced what follows.
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
                            Block::ToolUse { id, name, input } => {
                                Some(Event::Tool { id, name, input })
                            }
                            _ => None,
                        }),
                )
                .collect()
        }
        Line::User { message } => message
            .content
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                Block::ToolResult {
                    tool_use_id,
                    is_error,
                    content,
                } => {
                    let (detail, cut) = returned(content);
                    Some(Event::ToolResult {
                        id: tool_use_id,
                        ok: !is_error,
                        detail,
                        cut,
                    })
                }
                Block::Text { text } => match finished(&text) {
                    Some(event) => Some(event),
                    None if is_notification(&text) => None,
                    // A replayed prompt: the text this console sent, coming back.
                    None if !is_plumbing(&text) => Some(Event::Prompt { text }),
                    None => None,
                },
                _ => None,
            })
            .collect(),
        Line::Result(turn) => vec![Event::Turn {
            // Whichever model answered; there is one entry in practice, and the
            // largest is the honest answer if a turn ever spanned two.
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

/// The tool that asks the person a question rather than the machine a favour.
///
/// It is gated by `can_use_tool` like any other, so it arrives as an ordinary
/// [`Event::Ask`] — but approving it unchanged is not an answer. See [`Answers`].
pub const QUESTION_TOOL: &str = "AskUserQuestion";

/// What was chosen: the question's own text, against the option label picked.
///
/// One label for a single-choice question, several for a `multiSelect` one. The
/// CLI matches these against the labels it offered, so they are sent back
/// verbatim rather than by index — an index would silently mean the wrong option
/// if the list were ever reordered between asking and answering.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Answer {
    One(String),
    Many(Vec<String>),
}

/// Every answer to one [`QUESTION_TOOL`] call.
pub type Answers = std::collections::BTreeMap<String, Answer>;

/// Something said about one question, beside the option picked for it.
///
/// ⚠ **Unlike [`Reply::response`], this combines rather than overrides.** The
/// CLI reports `"<question>"="<label>" notes: <notes>`, and a question carrying
/// a note but no choice is still an answer — it reports that one as
/// `"<question>"=(no option selected) notes: …`. So a note is not a lesser
/// version of answering; it is the way to answer *and* qualify it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Annotation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

/// Notes against the questions of one call, by the question's own text.
pub type Annotations = std::collections::BTreeMap<String, Annotation>;

/// What a person said about a question: options picked, or words instead.
///
/// ⚠ **`response` and `answers` are alternatives, not companions.** The CLI's
/// result builder tests `response` first and reports only that, so prose sent
/// alongside a set of choices silently throws the choices away. Read off 2.1.220:
///
/// ```text
/// else if (response?.trim()) a = `The user responded: ${response}`
/// else if (s)               a = `The user answered: …`
/// ```
///
/// The client is where that is made visible — a card that offered both at once
/// would be offering one of them dishonestly.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Answers::is_empty")]
    pub answers: Answers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    /// Notes beside the choices — see [`Annotation`]. These travel *with*
    /// `answers`, not instead of them.
    #[serde(default, skip_serializing_if = "Annotations::is_empty")]
    pub annotations: Annotations,
}

impl Reply {
    /// Whether there is anything here to say. An empty reply is not an answer,
    /// and sending one would read to the session as "did not answer" — which is
    /// true, but is better refused at the door than discovered a turn later.
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
/// **An allow must carry the arguments back.** The protocol lets a client edit
/// what it is approving — that is what `updatedInput` is for — and the console
/// approves what was asked, so it echoes the input unchanged rather than
/// omitting it. A deny carries a message, which the session sees as the reason
/// and can act on.
///
/// **For [`QUESTION_TOOL`] that edit is the whole point.** Its `call` reads
/// `answers` out of its own arguments and formats them — it prompts nobody — so
/// a client answers by approving an input it has written the answers into.
/// Approving unchanged is what produces *"The user did not answer the
/// questions."*: the CLI builds its result from `answers`, and an absent one
/// falls through to that last branch. Measured against 2.1.220, and the reason
/// the console showed a question as allow/refuse for as long as it did.
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

/// The approved input, with the answers written into it.
///
/// Nothing is invented when there are none: the input goes back exactly as it
/// came, which is what an ordinary approval means. A non-object input is left
/// alone rather than replaced — there is nowhere to put an answer in it, and
/// dropping what the tool was asked to do would be worse than not answering.
fn answered(input: &serde_json::Value, reply: Option<&Reply>) -> serde_json::Value {
    let Some(reply) = reply else {
        return input.clone();
    };
    let mut input = input.clone();
    if let Some(object) = input.as_object_mut() {
        if !reply.answers.is_empty() {
            object.insert("answers".to_string(), serde_json::json!(reply.answers));
        }
        // Only when there is something in it: an empty string is falsy to the
        // CLI's `response?.trim()` test, but writing the key at all says a
        // choice was overridden by nothing.
        if let Some(said) = reply.response.as_deref().filter(|s| !s.trim().is_empty()) {
            object.insert("response".to_string(), serde_json::json!(said));
        }
        // Blank notes are dropped rather than sent: the CLI tests `notes` for
        // truthiness, and an empty one against an unanswered question would make
        // it report `(no option selected)` for a question nobody touched.
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

/// Ask the session to change what it may do without asking.
///
/// **This is a request going the other way.** Every other control message here
/// is the console *answering* something the CLI asked; this one the console
/// asks, on the same stdin it sends prompts on. Read off the 2.1.220 binary,
/// which sends exactly this shape from its own `setPermissionMode`.
///
/// The reply comes back as a `control_response` carrying `request_id`, which
/// this console does not wait for: the mode is a preference rather than a
/// transaction, and a client that blocked on it would freeze a session whose CLI
/// was busy. What it costs is knowing for certain the change landed — see
/// [`crate::session::Session::set_mode`].
pub fn set_mode(request_id: &str, mode: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "set_permission_mode", "mode": mode},
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
        Line::Assistant { message } => {
            // Same as the live reader: the context as it stood for this request,
            // ahead of what it produced. Without it a resumed or upgraded
            // session knows nothing about its own fullness until it finishes a
            // turn — and the numbers were in the transcript the whole time.
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
                            Block::ToolUse { id, name, input } => {
                                Some(Event::Tool { id, name, input })
                            }
                            _ => None,
                        }),
                )
                .collect()
        }
        Line::User { message } => message
            .content
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                Block::ToolResult {
                    tool_use_id,
                    is_error,
                    content,
                } => {
                    let (detail, cut) = returned(content);
                    Some(Event::ToolResult {
                        id: tool_use_id,
                        ok: !is_error,
                        detail,
                        cut,
                    })
                }
                Block::Text { text } => match finished(&text) {
                    Some(event) => Some(event),
                    None if is_notification(&text) => None,
                    None if !is_plumbing(&text) => Some(Event::Prompt { text }),
                    None => None,
                },
                _ => None,
            })
            .collect(),
        Line::System(System::CompactBoundary) => vec![Event::Compacted],
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
/// The tool call a task notification is about, and how it ended.
///
/// The harness files these as user messages — text, in the same place a typed
/// instruction lands. Read for the tool-use id rather than the task id, because
/// that is what ties it to the call that started it: a client counting what is
/// still running has the id from the `Tool` event and nothing else to match on.
///
/// Not a parser for the whole shape. Two tags out of a known block, and anything
/// that does not carry both is not one of these.
fn finished(text: &str) -> Option<Event> {
    if !is_notification(text) {
        return None;
    }
    Some(Event::Background {
        tool: between(text, "<tool-use-id>", "</tool-use-id>")?.to_string(),
        status: between(text, "<status>", "</status>")
            .unwrap_or("done")
            .to_string(),
    })
}

/// Whether this text is the harness reporting on a background task at all.
///
/// Separate from parsing it, because one that cannot be parsed still must not
/// become a prompt: a half-recognised notification rendered as somebody's words
/// is the exact failure this whole path exists to prevent.
fn is_notification(text: &str) -> bool {
    text.contains("<task-notification>")
}

/// What sits between two markers, when both are there and in that order.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim())
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
    TAGS.iter().any(|tag| head.starts_with(tag))
}
