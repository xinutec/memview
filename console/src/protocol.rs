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
    /// A message this console has written to the session's stdin, before the CLI
    /// has read it.
    ///
    /// ⚠ **The gap between this and [`Event::Prompt`] is minutes, not
    /// milliseconds, and nothing used to show it.** The CLI parks input that
    /// arrives mid-turn — it appears in the transcript as a `queued_command` —
    /// and releases it in batches: measured on 2026-08-07, four messages taken
    /// at 19:46:07, 19:46:22, 19:53:57 and 19:55:07 were all read at 19:57:59,
    /// the oldest after **twelve minutes**. Between turns the same trip is
    /// seconds.
    ///
    /// With only the echo to go on, a message sent to a working session left no
    /// trace at all until it was read, which is indistinguishable from a message
    /// that never arrived — so it gets sent again. Three duplicates in one
    /// evening, from the phone, by somebody who had every reason to believe the
    /// first had failed.
    ///
    /// The pair is deliberate: this says *the runner has it*, `Prompt` says *the
    /// session has read it*, and only both together describe the trip.
    Accepted {
        text: String,
    },
    /// The prompt, echoed back by `--replay-user-messages`. It is the CLI's own
    /// acknowledgement that the message arrived, which is worth showing: it is
    /// the difference between "sent" and "received" when the phone is on a
    /// train.
    Prompt {
        text: String,
    },
    /// A slash command — `/compact`, `/context` — rather than something said to
    /// the model.
    ///
    /// ⚠ **A command has no read receipt, and there is no way to give it one.**
    /// Measured 2026-08-08 against CLI 2.1.221, spawned with the same flags the
    /// runner uses: `/context` written to stdin produced `system`, a
    /// synthetic `assistant` and `result` on stdout, and **no user message at
    /// all**. `--replay-user-messages` does not replay a command. The transcript
    /// on disk does record it, as a `<command-name>` wrapper the CLI expanded it
    /// into, so the two readers see different things — the only place in this
    /// protocol where that is true.
    ///
    /// So this is what [`Event::Accepted`] would be if waiting were honest.
    /// Accepted promises an echo is coming; for a command none ever is, and a
    /// *waiting to be read* marker on one is a lie the console cannot stop
    /// telling — which it did, on `life`, throughout a compaction that was
    /// already running (memview #120).
    ///
    /// Its own variant rather than a `Prompt`, so that everything counting what a
    /// person said keeps counting what a person said: [`crate::past::material`]
    /// opens a summary with the first prompt, and `/exit` is not what the
    /// conversation was about.
    Command {
        text: String,
    },
    /// A picture that was sent to this session, by the name of the copy kept for
    /// it — enough for a reader to ask for it back at
    /// `/api/sessions/{id}/images/{name}`.
    ///
    /// ⚠ **The bytes are deliberately not here.** The transcript line holds the
    /// whole image as base64, and passing that through would put a megabyte on
    /// the wire every time somebody scrolled past it, in an event stream whose
    /// whole design is small messages. The file is already on disk; a name is all
    /// anyone needs to ask for it.
    Shown {
        name: String,
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
    /// A rate-limit window changed state.
    ///
    /// ⚠ **This carries the percentage, and for a long time we believed it did
    /// not.** The comment here used to say the figure existed only in the
    /// statusLine hook's input, and the console went and read it off the home
    /// dashboard instead — hours stale, because a status line belongs to a
    /// terminal and these sessions are headless. The CLI's own schema for
    /// `rate_limit_event` says otherwise: `status`, `resetsAt`, `rateLimitType`
    /// **and `utilization`**. It was arriving on this pipe all along and being
    /// dropped for want of a field to put it in.
    ///
    /// One window per event — the *representative* one, which the API names in
    /// an `anthropic-ratelimit-unified-representative-claim` header — so the
    /// windows are collected as they are seen rather than all at once.
    Limit {
        window: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        resets_at: Option<i64>,
        /// How much of the window is spent, as a fraction. Optional in the CLI's
        /// schema, so optional here.
        #[serde(skip_serializing_if = "Option::is_none")]
        utilization: Option<f64>,
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
        /// The call this is asking about — the `tool_use` id, which is also on
        /// the [`Event::Tool`] the CLI emitted a moment earlier.
        ///
        /// ⚠ **Without it one action draws two widgets.** The CLI announces the
        /// call and then asks about it, so a client that cannot tell they are
        /// the same thing shows a tool row AND a permission card for one Write —
        /// and, worse, the card between two calls breaks the run they would
        /// otherwise fold into. Measured 2026-08-11: `tool toolu_01E9WgUY…`
        /// followed by `ask c8471a53-…` carrying identical input.
        ///
        /// Optional because not every call site sends it — the CLI has three
        /// that build this request and one omits it — so a client must still
        /// cope with an ask it cannot attach to anything.
        #[serde(skip_serializing_if = "Option::is_none")]
        call: Option<String>,
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
    /// The session has stopped reading its stdin: messages were written to it,
    /// it is between turns, and it has not taken them.
    ///
    /// Announced once per episode rather than repeated, and never read from a
    /// transcript — it is the console's own conclusion about a process, not
    /// something the conversation records. See [`crate::session::Session::deaf`]
    /// for what it is concluded from and what it cannot see.
    Deaf {
        /// How many messages are waiting in the pipe.
        unread: usize,
        /// How long it has been failing to take them, in seconds.
        seconds: u64,
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
    /// The CLI parking a message it has been handed but has not read yet.
    ///
    /// ⚠ **The earliest record that background work has ended, and by minutes.**
    /// A notification is enqueued the moment the work finishes and dequeued only
    /// when the turn in progress lets go — measured at `enqueue` 11:55:45,
    /// `remove` 11:58:56, three minutes during which the only evidence on this
    /// machine was this line. The `user` message it eventually becomes is written
    /// at the far end of that gap, so waiting for it means a card that says work
    /// is running for as long as the session keeps talking.
    ///
    /// Kebab-case on the wire, unlike every other line here, so it is renamed
    /// rather than derived.
    #[serde(rename = "queue-operation")]
    QueueOperation {
        operation: String,
        #[serde(default)]
        content: Content,
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

/// A question from the CLI. Only one subtype is answered here; the rest are for
/// clients that offer more than this one does.
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
    /// Which model answered — read only to recognise the one that is not a
    /// model at all. See [`SYNTHETIC`].
    #[serde(default)]
    model: Option<String>,
}

/// The `model` a message carries when nothing generated it.
///
/// ⚠ **This is how a slash command answers, and the live reader threw it away.**
/// A command runs locally and its output arrives as one complete `assistant`
/// message — no `content_block_delta` before it, because nothing was generated.
/// [`read`] keeps only tool calls from a completed message, on the sound rule
/// that the deltas already carried the text, and that rule is wrong about the
/// one case that has no deltas: **every** slash command's output was silently
/// dropped, not only `/tasks` (memview #106).
///
/// Measured 2026-08-08 against CLI 2.1.221: `/context` on a session's stdin came
/// back as `"model": "<synthetic>"` carrying the whole context table.
const SYNTHETIC: &str = "<synthetic>";

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
/// What one event says about work that was left running.
///
/// A decision rather than a mutation, so the four cases can be tested without a
/// session, a process or a lock — and so the one that is easy to forget is
/// written down as a case rather than buried in an `if`.
#[derive(Debug, PartialEq, Eq)]
pub enum Running {
    /// A call was started and left going, named by its own id. The call itself
    /// returns at once with a task id and nothing else.
    ///
    /// Both names are kept because the two endings speak different ones: a
    /// notification names the `tool` that started the work, a kill names the
    /// `task` the harness gave it. `task` is `None` for a detach whose id could
    /// not be read — still running, merely not matchable to a kill.
    Began { tool: String, task: Option<String> },
    /// The harness says one has finished, naming the call that started it.
    Ended(String),
    /// A task was stopped from here, named by its task id. Killing one is the
    /// other way work ends, and the only way that reports nothing afterwards.
    Killed(String),
    /// Nothing that was running still is — as far as anybody can tell.
    Gone,
    /// This event says nothing about background work.
    Quiet,
}

/// Read one event for what it says about work left running.
///
/// ⚠ **One event means "forget what you were counting", and it is not the
/// obvious one.** A `Joined` is the boundary between the transcript that was
/// replayed and the stream being watched: the replay is full of calls that were
/// backgrounded hours ago, and counting them said five tasks were running on a
/// session whose newest was nine hours gone and which had no children at all.
/// It also covers the restart it looks like it does not: a re-seeded session
/// replays that same history and then joins, so the phantoms a dead process left
/// behind are cleared at the same boundary.
///
/// ⚠ **`Started` was here and was wrong.** An init line is not a new process —
/// the CLI emits one at the head of *every turn*, measured on this console's own
/// stream as `turn → started → busy → prompt`, while the session's cost, context
/// and children all carry on. Reading it as a restart emptied the count on the
/// next message sent, which is exactly when it is worth having: a commit sat in
/// the gate for eight minutes with the card claiming nothing was running.
///
/// ⚠ **A killed task is never reported finished, so the kill has to be read
/// here.** Stopping one answers on the *stopping* call — the started call has
/// already returned — and no notification ever follows. Measured over this
/// machine's transcripts: 162 kills, not one of them notified. A watcher started
/// and stopped thirteen seconds later held the count at one for the rest of the
/// session.
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
        Event::Background { tool, .. } => Running::Ended(tool.clone()),
        Event::Joined { .. } => Running::Gone,
        _ => Running::Quiet,
    }
}

/// What a tool says when it has left work running.
///
/// ⚠ **The call's answer, not its arguments — and this is the whole of the
/// decision about what counts.** It used to be `run_in_background: true` on the
/// input, which is a *request* to detach and which only `Bash` accepts. Measured
/// across 27,731 calls in one 241 MB transcript: that flag appears on `Bash` and
/// on nothing else, so a `Monitor` running for twenty-five minutes counted as
/// nothing, and the card said the session had nothing going on.
///
/// Every tool that detaches says so in the first words it returns, and those
/// words are the CLI's, not ours. Measured against the same corpus — 13,858
/// calls whose result is known, of which 510 were later followed by a
/// task-notification:
///
/// - **495** carried one of these phrases and were notified.
/// - **8** carried one and had no notification yet, which is what a task still
///   running looks like at the end of a file.
/// - **15** were notified with no phrase: 13 `SendMessage` replies, whose result
///   is JSON with nothing to match on, and 2 more of the timeout kind below.
/// - **13,340** had neither. **No call matched a phrase without the work being
///   real** — the precision that matters, since the failure to avoid is a count
///   that never comes down.
///
/// ⚠ **The timeout phrase is the one no rule about arguments could have found.**
/// A foreground command that outlives its timeout is moved to the background by
/// the harness — thirteen of them in that transcript — and its input says
/// `run_in_background: false`, because that is what was asked for.
///
/// ⚠ **Matching prose is not free and the shape of the risk decides it.** These
/// are English sentences and a reworded CLI stops matching. But an unmatched
/// phrase undercounts, which is exactly today's behaviour and is visibly wrong
/// in one direction only; a rule that guessed from the tool's *name* would count
/// work that never started and leave the number stuck at one for the life of the
/// session. Failing closed is worth a fragile match.
///
/// ⚠ **The phrase has to *open* the result, not merely appear in it — because
/// otherwise reading this file starts a task.** A `contains` counted every
/// result that *quoted* one of these sentences: a grep for the phrase, a `Read`
/// of this module, a `Read` of the test that lists the openings verbatim. The
/// console inflated its own count whenever anyone opened the rule that defines
/// it. Measured over this machine's transcripts: 7,416 results matched
/// anywhere, 7,405 at the front, and all 11 of the difference were quotations —
/// no genuine launch announces itself in the middle of a sentence.
///
/// Returns the task id the harness gave the work, since a kill names that and
/// not the call. `Some(None)` is a detach whose id could not be read: still
/// running, merely not matchable to a kill. It was readable on all 7,405.
fn detached(said: &str) -> Option<Option<String>> {
    /// One way a result opens when it has left work running: the words it starts
    /// with, and the marker its task id follows.
    struct Opening {
        opens: &'static str,
        before_id: &'static str,
    }
    const SAYS: [Opening; 4] = [
        // Bash, asked to detach.
        Opening {
            opens: "Command running in background with ID: ",
            before_id: "Command running in background with ID: ",
        },
        // Bash, not asked to, and moved anyway when it outran its timeout. The
        // opening stops short of the id because the timeout itself — a number
        // that varies by call — sits between the two.
        Opening {
            opens: "Command did not complete within its",
            before_id: "and was moved to the background (ID: ",
        },
        // Agent, which runs in the background unless told otherwise. Its id is
        // on the following line, and the result asks that it never be repeated
        // to anyone — so it is matched on here and rendered nowhere.
        Opening {
            opens: "Async agent launched successfully",
            before_id: "\nagentId: ",
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

/// The task a kill has just ended, when this result is one.
///
/// The stopping call answers in JSON rather than prose, which is why this is a
/// prefix of the whole document and not a sentence: `{"message":"Successfully
/// stopped task: …`. Only the one tool says it.
fn stopped(said: &str) -> Option<String> {
    id_at(said.strip_prefix(r#"{"message":"Successfully stopped task: "#)?)
}

/// The id starting here, which runs until whatever ends it — a full stop, a
/// comma, a bracket or a space, depending on which sentence it was found in.
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
    /// A fraction, not a percentage — the CLI multiplies by 100 on its way to
    /// the status line, and so does the console on its way to a screen.
    #[serde(default)]
    utilization: Option<f64>,
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
        // Also a file-only line, and for the same reason as the boundary above:
        // what the CLI parks for itself it does not announce. It is read off the
        // transcript instead — see [`crate::past::Appended::finished`].
        Line::QueueOperation { .. } => Vec::new(),
        Line::StreamEvent { event } => match event {
            Stream::ContentBlockDelta { delta } => match delta {
                Delta::Text { text } => vec![Event::Text { text }],
                Delta::Other => Vec::new(),
            },
            Stream::Other => Vec::new(),
        },
        // Text is taken from the deltas, so the completed message contributes
        // only what the deltas cannot carry whole — unless nothing generated it,
        // in which case there were no deltas and this is the only copy. See
        // [`SYNTHETIC`].
        Line::Assistant { message } => {
            // The context as it stood for THIS request, ahead of the blocks, so
            // a reader sees the fullness that produced what follows.
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
                            Block::ToolUse { id, name, input } => {
                                Some(Event::Tool { id, name, input })
                            }
                            Block::Text { text } if spoken && !text.trim().is_empty() => {
                                Some(Event::Text { text })
                            }
                            _ => None,
                        }),
                )
                .collect()
        }
        // A replayed prompt: what this console sent, coming back — and read by
        // the same function that reads one out of a transcript, so a picture
        // appears on screen when it is sent rather than only when the
        // conversation is next opened.
        Line::User { message } => from_user(message.content),
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
/// Rename a conversation, over the CONTROL channel.
///
/// ⚠ **This is the only way to rename a session that is working**, and the
/// reason is the channel rather than the wording. `/rename` is *input*: written
/// to stdin, and the CLI parks input that arrives mid-turn and releases it as a
/// **prompt** — `commandMode: "prompt"`, which is what all 1,756 queued messages
/// in this machine's transcripts are. So the model reads the words `/rename
/// tasks` and the command never runs. Measured 2026-08-08 on a session that was
/// mid-turn: the agent replied "Noted the rename (CLI-side, nothing for me to
/// do)" and no `custom-title` line was ever written.
///
/// A control request is out-of-band and is handled whatever the turn is doing.
/// Measured against 2.1.226, sent two seconds into a running turn: `success`
/// came back at once and the transcript gained
/// `{"type":"custom-title","customTitle":…}` — which is the first field in the
/// console's own naming chain, so the new name is on the list at the next poll.
///
/// ⚠ **`title`, and it must be a string** — the CLI's own validation message.
/// The subtype is also refused outright by hosts that register no rename
/// callback ("not supported in this context"); a `-p` session does support it,
/// which is what the probe above established before this was written.
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

/// Ask the session what the account has spent.
///
/// **The only route to the routine figures**, and it took some finding. The
/// numbers ride on every API response as `anthropic-ratelimit-unified-*`
/// headers, and the CLI keeps them — but it publishes them in just two places:
/// to a `statusLine` command, which is a terminal's affair and never runs for a
/// headless session, and onto the stream as `rate_limit_event`, which carries
/// the percentage **only when a threshold is crossed** (≥90% of a window with
/// ≤72% of its time gone). Normal operation reports status and reset time and no
/// figure at all — measured on a live stream.
///
/// `get_usage` is the CLI's own answer to the question: one control request, and
/// both windows come back with `utilization` and `resets_at`. The CLI describes
/// it as experimental and says the shape may change, which is why nothing here
/// insists on it — see [`usage_reply`].
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
/// ⚠ **Read defensively, field by field, and deliberately so.** The CLI calls
/// `get_usage` experimental and warns that the response shape may change, so
/// this reads the two windows it wants and ignores everything else — the cost,
/// the per-model buckets, the overage blocks. A shape that has moved yields no
/// reading rather than a wrong one, and the front page goes back to the
/// dashboard.
///
/// Not matched against a request id: the console asks for nothing else, so any
/// response carrying rate limits is an answer to this.
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
        // `utilization` is a percentage here, where the stream event's is a
        // fraction. Both are read into a fraction, so one thing downstream
        // multiplies by a hundred rather than two things disagreeing about
        // whether it has been done already.
        let Some(pct) = seen.get("utilization").and_then(|it| it.as_f64()) else {
            continue;
        };
        let resets_at = seen
            .get("resets_at")
            .and_then(|it| it.as_str())
            .and_then(|it| OffsetDateTime::parse(it, &Rfc3339).ok())
            .map(|when| when.unix_timestamp());
        found.push((window.clone(), pct / 100.0, resets_at));
    }
    (!found.is_empty()).then_some(found)
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

/// Everything one user message says, live or read back off the disk.
///
/// One function for both readers because a user message is one shape wherever it
/// is met — the live one is the CLI replaying what this console just sent, and
/// the recorded one is the same message written down. Two copies of this drifted
/// apart once already.
///
/// ⚠ **Read across the blocks, not within one.** A sent picture is two blocks —
/// the image, then the words — and the image block is the only one that says a
/// picture is here while the text block is the only one that says *which*.
/// Neither answers alone, which is why this is not the plain per-block mapping
/// the rest of the reader is.
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
                let (detail, cut) = returned(content);
                vec![Event::ToolResult {
                    id: tool_use_id,
                    ok: !is_error,
                    detail,
                    cut,
                }]
            }
            Block::Text { text } if carries => {
                let (name, words) = shown(&text);
                // The picture first and the words after, in the order they were
                // sent and for the same reason: a question reads as being about
                // the thing above it.
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
        .fold(Vec::new(), one_thing_said)
}

/// One message is one thing said, however many blocks it arrived in.
///
/// ⚠ **Measured, and it made somebody doubt their own memory.** A prompt reached
/// the CLI twice within a millisecond and was merged into a single message
/// carrying the same words in two blocks — which this reader turned into two
/// prompts, so the transcript showed a question asked twice that was asked once.
/// One in every user message in this project's transcripts, so the merging is
/// rare; the misattribution it caused is not the kind worth leaving in.
///
/// Blank line between, because separate blocks are separate paragraphs — that is
/// how the model was given them.
fn one_thing_said(mut said: Vec<Event>, event: Event) -> Vec<Event> {
    match (said.last_mut(), event) {
        (Some(Event::Prompt { text: held }), Event::Prompt { text }) => {
            held.push_str("\n\n");
            held.push_str(&text);
        }
        (_, event) => said.push(event),
    }
    said
}

/// The phrase that ties a sent picture to the copy on disk. Written by
/// [`prompt_with_image`], read by [`shown`], and of no interest to anyone else.
const ALSO_AT: &str = "the image is also at ";

/// The picture a user message carries, and the words with the note about it
/// taken out.
///
/// The note is addressed to the session — it is how a model that has had the
/// image compacted away can open the file again — and it is plumbing to anybody
/// reading the conversation, who is about to be shown the picture itself. So the
/// reader gets the words without it, and an empty result means the picture was
/// sent with nothing said.
///
/// Only the file name comes back, never the path it was found in: the client
/// asks for a picture by session and name, and a directory it cannot use is a
/// directory it has no reason to be told.
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

/// One user message carrying a picture and what was said about it.
///
/// ⚠ **Measured against CLI 2.1.221 before it was built on.** The CLI takes an
/// `image` block on stdin exactly as the API defines one, forwards it, and the
/// model reads it — a screenshot sent this way came back described. Nothing in
/// the CLI's documented input schema promised that.
///
/// **The picture first, the words after.** Anthropic's own guidance for a single
/// image, and the difference is not cosmetic: a question read before the thing it
/// is about is answered from the question alone.
///
/// The text also names where the console kept its copy. The image itself lives in
/// the conversation only until it is compacted away, and a session asked about it
/// an hour later has no way back to it — with the path, it can simply open the
/// file, at the size it was sent rather than the size that was sent.
///
/// ⚠ **That note is also how the picture is found again when the transcript is
/// read back.** A recorded image block holds base64 and no name, so the only
/// thing on the line that says *which* picture this was is the path in the words
/// beside it. [`shown`] is the other half of this function and the two share
/// [`ALSO_AT`]; changing the sentence here without changing that one leaves every
/// picture already sent unreadable.
///
/// Wordless and worded messages carry the same note for the same reason — one
/// shape to write, one shape to read.
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
        Line::User { message } => from_user(message.content),
        // Only what has just been handed over, and only if it is a notification.
        // `remove` is the same message going the other way — reading it too would
        // be the same finish twice, and `enqueue` is the one that happens when
        // the work actually ended.
        Line::QueueOperation { operation, content } if operation == "enqueue" => content
            .blocks()
            .into_iter()
            .filter_map(|block| match block {
                Block::Text { text } => finished(&text),
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
///
/// ⚠ **The block has to be the end of the message, not merely somewhere in it.**
/// This was a bare `contains`, and it ate the compact summary — a message that
/// happened to explain what a notification looks like, so the one turn that
/// says what the conversation was about vanished from the screen.
///
/// Both ends rather than the opening tag alone, because the tag is not always
/// first: the harness prefixes a banner saying this is not the person talking.
/// Measured over a 267 MB transcript — 1,385 notifications, every one of them
/// closing the block as its last characters.
fn is_notification(text: &str) -> bool {
    text.contains("<task-notification>") && text.trim_end().ends_with("</task-notification>")
}

/// What sits between two markers, when both are there and in that order.
fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text[start..].find(close)? + start;
    Some(text[start..end].trim())
}

/// The slash command a message typed into the console is, if it is one.
///
/// The console has to decide this on the way *in*, before anything comes back,
/// because what comes back differs: a prompt is echoed and a command is not.
/// See [`Event::Command`].
///
/// **A leading slash and then a word.** That is the CLI's own rule, and the
/// second half of it is doing work: `/Users/pippijn/Code/…` opens with a slash
/// and is a path somebody pasted, so the word must run to whitespace or to the
/// end. Getting this wrong costs a read receipt on an ordinary message, never a
/// false one on a command.
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
/// typed.
///
/// The wrapper is the only trace a command leaves, and it is the CLI's, not the
/// person's: `<command-name>` with the slash, `<command-args>` with whatever
/// followed. Both halves matter — `/loop check eval output` and `/loop` are two
/// different messages, and only the whole of it matches what the console sent.
///
/// Recognised by how the block opens, so that asking *what does
/// `<command-name>` mean* stays a question. ⚠ Both openings occur: measured over
/// this machine's transcripts, 1,369 lead with the name and 95 lead with the
/// message and carry no args at all.
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
    TAGS.iter().any(|tag| head.starts_with(tag))
}

/// A background call, named the way somebody looking at the strip would name it.
///
/// ⚠ **The count was never the question.** `Summary::background` reported
/// `state.background.len()`, so a phone showing *1 background task running*
/// could not say which — and answering it took a `ps` on the Mac (memview #740).
/// Both halves were already in memory: the call that started the work and the
/// task id the harness gave it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Called {
    /// The tool, as the CLI names it: `Bash`, `Monitor`, `Agent`.
    pub tool: String,
    /// A short human label for what this particular call is doing, when the
    /// input carries one. `None` when it does not — better an unlabelled tool
    /// name than a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// The harness's task id, which is what a kill names.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// How long a label may be before it is cut.
///
/// ⚠ **Measured, not chosen for looks.** A `Bash` background task's label on
/// this machine ran to several hundred characters of shell one-liner, because
/// the tool's own `description` is optional and the fallback is the command. The
/// strip has one line on a 412 px phone; the untruncated text belongs in the
/// sheet, which can scroll.
const LABEL_MAX: usize = 60;

/// Read a tool call for the name and label a person would use for it.
///
/// The label is the tool's OWN description where it has one — `Bash` and
/// `Monitor` both take one and it is written for a human — falling back to the
/// field that carries the work. Nothing is invented: a call whose input says
/// nothing readable gets no label rather than a rendering of its JSON.
pub fn called(tool: &str, input: &serde_json::Value) -> Called {
    let pick = |key: &str| {
        input
            .get(key)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    // `description` first for every tool that has one: it is the sentence the
    // caller wrote about this call. `command`/`prompt` are the work itself.
    let label = pick("description")
        .or_else(|| pick("command"))
        .or_else(|| pick("prompt"))
        .map(|text| {
            // Collapse newlines: a heredoc would otherwise take the strip down
            // the page, and only the first line is legible there anyway.
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
