//! The mutable half of a session and how each event changes it.

use std::collections::{BTreeMap, VecDeque};

use super::{Heard, ResetsAt, Seen};
use crate::protocol::{self, Event};

/// How much transcript one session keeps in memory: a scrollback, not an archive.
const SCROLLBACK: usize = 5000;

/// How many recent tool calls are remembered so a detached one can be named.
/// Small: the `Tool` event is immediately followed by the `ToolResult` that
/// reveals the detach, and this is held for the life of a process that runs for days.
const CALLED_RING: usize = 32;

/// An event and its place in the session's order. The number is what makes a
/// dropped connection survivable: a reconnecting client says what it has and is
/// sent what it missed. Sequential from 1, per session, assigned under the lock
/// that appends to the log.
#[derive(Debug, Clone)]
pub struct Stamped {
    pub seq: u64,
    /// When it happened, in milliseconds since the epoch. See [`protocol::Timed`].
    pub at: Option<i64>,
    pub event: Event,
}

/// What a connecting client is owed, and whether what it already has is good.
#[derive(Debug)]
pub struct Backlog {
    /// False means the client's page cannot be kept: it named nothing, or an event
    /// this session no longer holds.
    pub resumed: bool,
    pub events: Vec<Stamped>,
    /// The highest sequence number this client has now been sent. Live events at or
    /// below it must not be sent again.
    pub through: u64,
}

/// Whether a client holding everything through `after` can be sent only what it
/// missed, given a log holding `held_from..=issued`. Older than the log's front
/// means the scrollback dropped the difference; newer than anything issued means
/// another session's numbering. `after + 1` on the left: the next event is what
/// has to be reachable, so an exactly-caught-up client resumes against an empty log.
pub fn resumable(after: u64, held_from: u64, issued: u64) -> bool {
    after + 1 >= held_from && after <= issued
}

/// A question the session is waiting on an answer to. The arguments are kept
/// because an allow has to echo them back, and the tool's name because only one
/// tool's arguments may be edited — see [`protocol::QUESTION_TOOL`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    pub tool: String,
    /// The call being asked about — the `tool_use` id, carried across an upgrade so a
    /// re-seeded question can still say which tool row it belongs to.
    #[serde(default)]
    pub call: Option<String>,
    pub input: serde_json::Value,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

impl Pending {
    /// The question again, as the event a client is offered it by.
    pub fn ask(self, id: String) -> Event {
        Event::Ask {
            id,
            does: crate::call::Call::read(&self.tool, &self.input),
            call: self.call,
            tool: self.tool,
            title: self.title,
            detail: self.detail,
            input: self.input,
        }
    }
}

/// The mutable half of a session, behind one lock.
#[derive(Debug, Default)]
pub(super) struct State {
    pub(super) log: VecDeque<Stamped>,
    /// The last sequence number issued. Not the log's length: the log drops its
    /// front, and a reused number would resume a client into the wrong place.
    pub(super) issued: u64,
    /// Questions the session is blocked on, by control-request id.
    pub(super) pending: BTreeMap<String, Pending>,
    pub(super) alive: bool,
    /// When the kill armed by [`Session::stop`] falls due, in epoch milliseconds;
    /// `None` for a session nobody has stopped. Read by [`crate::roster::Roster::handover`].
    pub(super) stopping: Option<i64>,
    pub(super) model: Option<String>,
    pub(super) busy: Option<String>,
    /// See [`Summary::interactions`], and [`crate::past::counted`] for why the byte
    /// offset travels with the number.
    pub(super) counted: crate::past::Counted,
    /// See [`Summary::mode`]. Written optimistically when the console asks for a
    /// change, and corrected when the CLI answers — [`Session::settle_mode`].
    pub(super) mode: Option<crate::modes::Mode>,
    /// What the mode was before a change the CLI has not answered yet, so a refusal
    /// can put it back. Cleared either way when the answer arrives.
    pub(super) restore: Option<crate::modes::Mode>,
    /// See [`Summary::mode_refused`].
    pub(super) mode_refused: Option<String>,
    pub(super) cost_usd: f64,
    /// The last turn's prompt size and the window it went into. See
    /// [`Summary::context`].
    pub(super) context: Option<u64>,
    pub(super) window: Option<u64>,
    /// See [`Summary::limit`].
    pub(super) limit: Option<crate::protocol::Allowance>,
    /// What the API last said about each rate-limit window. See [`Tally::spent`].
    pub(super) spent: BTreeMap<String, Seen>,
    /// Background tool calls started and not yet ended, keyed by the call (what a
    /// notification names), carrying the task (what a kill names).
    pub(super) background: std::collections::BTreeMap<String, crate::protocol::Called>,
    /// The last few tool calls seen, by call id, so a background one can be named
    /// when its result arrives. A ring: an unbounded map would hold every call of a
    /// session that runs for days.
    pub(super) called: std::collections::VecDeque<(String, crate::protocol::Called)>,
    /// When the process last wrote a line, in epoch milliseconds — [`Session::heard`].
    pub(super) heard: i64,
    /// Whether the transcript has been consulted for the session's origin —
    /// "there is no origin" against "we have not looked", which `asked: None` cannot
    /// say. A conversation continued from a compacted one correctly has none, and the
    /// next prompt must not be taken for it. Only a session with no transcript may
    /// be named by what it is told next.
    pub(super) origin_read: bool,
    pub(super) asked: Option<String>,
    pub(super) stderr: String,
    /// Messages written to stdin that the CLI has not echoed back, oldest first. See
    /// [`Session::deaf`].
    pub(super) unread: VecDeque<Unread>,
    /// Whether the session has spoken since its last turn ended — [`Summary::working`].
    /// A positive fact, not `idle_since.is_none()`, which is true for a session that
    /// has said nothing at all and so reports a still-loading one as working.
    pub(super) working: bool,
    /// When the last turn ended, in epoch milliseconds — `None` whenever the session
    /// is working. See [`Session::deaf`].
    pub(super) idle_since: Option<i64>,
    /// Whether this episode of deafness has already been announced. See
    /// [`Session::check_deaf`].
    pub(super) announced_deaf: bool,
    /// When the oldest decision the session has not acted on was written, in epoch
    /// milliseconds. See [`Session::deaf`].
    pub(super) decided: Option<i64>,
    /// Slash commands written while a turn was running, oldest first. A command sent
    /// mid-turn does not run: the CLI parks it as a `queued_command` with
    /// `commandMode: "prompt"` and hands it to the model as words. Held here rather
    /// than in the client, which stops holding it when the phone is put away. See
    /// [`Session::send`] and [`Session::release_held`].
    pub(super) held: VecDeque<String>,
    /// A `/compact` has been sent and the conversation has not moved since — the one
    /// long silence that is not a fault. See [`Session::deaf`].
    pub(super) compacting: bool,
}

/// How long a message may sit unread, between turns and with the session
/// otherwise silent, before the session is called deaf. The legitimate wait is
/// seconds; a deaf session stays silent for tens of minutes. Ninety seconds sits
/// an order of magnitude clear of each.
pub(super) const DEAF_AFTER_MS: i64 = 90_000;

/// The same wait while a compaction is outstanding, which freezes the transcript
/// for minutes. Longer rather than suppressed: a session can go deaf around a
/// compaction.
pub(super) const DEAF_AFTER_COMPACT_MS: i64 = 15 * 60_000;

/// Drop one piece of background work from the count, by whichever name the thing
/// that ended it knew: a removal for a call, a search for a task.
pub(super) fn forget(
    background: &mut std::collections::BTreeMap<String, crate::protocol::Called>,
    named: &crate::protocol::Named,
) {
    match named {
        crate::protocol::Named::Call(call) => {
            background.remove(call);
        }
        crate::protocol::Named::Task(task) => {
            background.retain(|_, started| started.task.as_deref() != Some(task.as_str()));
        }
    }
}

/// Keep track of what is in flight, and whether the session can read it.
/// Everything [`Session::deaf`] decides on is maintained here, in one place: the
/// verdict is a conjunction, and a field updated in only some arms fails silently.
pub(super) fn in_flight(state: &mut State, event: &Event, now: i64) {
    match event {
        // The read receipt. Oldest match first: stdin is a queue.
        Event::Prompt { text } => {
            if let Some(at) = state.unread.iter().position(|held| &held.text == text) {
                state.unread.remove(at);
            }
            // It read something, so this episode is over; a next one is worth saying again.
            state.announced_deaf = false;
        }
        // A turn ended, so from here the session owes us a read.
        Event::Turn { .. } => state.idle_since = Some(now),
        // Nothing is running as of now. `Joined` matters most: pushed after the seeded
        // transcript, it stops a file ending mid-turn from reading as a turn still
        // going in a process that has only just started.
        Event::Started { .. } | Event::Joined { .. } => state.idle_since = Some(now),
        _ => {}
    }
    state.working = working_after(state.working, event);
    match event {
        Event::Command { text } if text.starts_with("/compact") => state.compacting = true,
        // A decision written down the pipe of a session that asked for it. Its own
        // clock: a session blocked on a question is mid-turn, so the message test
        // cannot fire. Silence after this is not work.
        Event::Answered { .. } => state.decided = state.decided.or(Some(now)),
        _ => {}
    }
    // Anything the session says of its own accord means it is working. Not `Busy`:
    // a status is announced only when it changes. A `Turn` counts but must not
    // clear `idle_since`, which it has just set.
    if matches!(
        event,
        Event::Text { .. }
            | Event::Tool { .. }
            | Event::ToolResult { .. }
            | Event::Context { .. }
            | Event::Prompt { .. }
            | Event::Turn { .. }
    ) {
        state.compacting = false;
        // Taken up: the tool it asked about has run, or the turn moved on.
        state.decided = None;
        if !matches!(event, Event::Turn { .. }) {
            state.idle_since = None;
        }
    }
}

/// Whether a turn is running, after `event`. Set by the session speaking,
/// cleared when the turn ends, and false until it has ever spoken. `Started` and
/// `Joined` clear it too, or a resumed session whose file ends mid-turn reads as
/// working. Public because reaching that case in a test needs only this.
pub fn working_after(was: bool, event: &Event) -> bool {
    match event {
        Event::Text { .. }
        | Event::Tool { .. }
        | Event::ToolResult { .. }
        | Event::Context { .. }
        | Event::Prompt { .. } => true,
        Event::Turn { .. }
        | Event::Exited { .. }
        | Event::Started { .. }
        | Event::Joined { .. } => false,
        // Everything else — a status, a decision, a command — leaves the answer where it was.
        _ => was,
    }
}

/// The verdict itself, against state a caller is already holding — see
/// [`Session::deaf`].
pub(super) fn deaf_for(state: &State, now: i64) -> Option<i64> {
    if !state.alive {
        return None;
    }
    deaf_after(
        state.idle_since,
        state.unread.front().map(|held| held.at),
        state.decided,
        state.compacting,
        now,
    )
}

/// The verdict as arithmetic. Public because it is the part worth testing without
/// waiting ninety seconds.
///
/// * `idle_since` — when the last turn ended, `None` while the session works.
/// * `oldest` — when the oldest unread message was written.
/// * `decided` — when the oldest unacted-on decision was written; needs no
///   `idle_since`, since the session asked and stopped.
///
/// Two ways to be waiting, and either is enough; whichever has waited longer is
/// reported.
pub fn deaf_after(
    idle_since: Option<i64>,
    oldest: Option<i64>,
    decided: Option<i64>,
    compacting: bool,
    now: i64,
) -> Option<i64> {
    // The later of the two: only once the turn has ended and the message has
    // arrived has the session had the chance this measures.
    let unread_since = idle_since.zip(oldest).map(|(idle, at)| idle.max(at));
    // The earlier of the two cases, so a long wait is not hidden by a short one.
    let since = [unread_since, decided].into_iter().flatten().min()?;
    let allowed = if compacting {
        DEAF_AFTER_COMPACT_MS
    } else {
        DEAF_AFTER_MS
    };
    let waited = now - since;
    (waited >= allowed).then_some(waited)
}

/// A message written to the session's stdin that it has not read back:
/// [`Event::Accepted`] says the bytes reached the pipe, the CLI's replay says
/// they were taken out. Commands are absent — the CLI never replays one.
#[derive(Debug, Clone)]
pub(super) struct Unread {
    pub(super) text: String,
    /// When it was written, in epoch milliseconds.
    pub(super) at: i64,
}

impl State {
    /// Fold one event into the state and give it its place in the order. `now` is
    /// the time to date a measurement by when the event carries none, which marks it
    /// as an echo rather than a measurement.
    pub(super) fn take(&mut self, event: Event, at: Option<i64>, now: i64) -> Stamped {
        match &event {
            Event::Started { model, .. } => self.model = Some(model.clone()),
            Event::Busy { status } => self.busy = Some(status.clone()),
            // The window, which only the result line declares.
            Event::Context { tokens } => self.context = Some(*tokens),
            // Everything the last measurement counted was replaced by a summary, so this
            // is another conversation's number. Cleared rather than estimated; the next
            // message brings a real one. Seen only in a replay — live, [`super::Session::recount`]
            // reads the file.
            Event::Compacted => self.context = None,
            Event::Turn {
                cost_usd, window, ..
            } => {
                if window.is_some() {
                    self.window = *window;
                }
                self.busy = None;
                // Assigned, not added: `total_cost_usd` is already the running total.
                self.cost_usd = *cost_usd;
                // `num_turns` is deliberately not read: it counts assistant messages, not
                // exchanges. [`super::Session::recount`] answers that from the file.
            }
            Event::Limit {
                window,
                status,
                resets_at,
                utilization,
            } => {
                self.limit = Some(status.clone());
                // One window per event, so they are collected rather than replaced wholesale.
                if let Some(spent) = utilization {
                    // A measurement: the API's own headers off a request that just completed, dated
                    // by the event's stamp on replay and `now()` live. An event with no stamp
                    // cannot be dated, which is the definition of an echo.
                    crate::usage::remember(
                        &mut self.spent,
                        [(
                            window.clone(),
                            Seen {
                                utilization: *spent,
                                resets_at: resets_at.map(ResetsAt),
                                at: Heard(at.unwrap_or(now)),
                                measured: at.is_some(),
                            },
                        )],
                    );
                }
            }
            Event::Prompt { text } if self.asked.is_none() && !self.origin_read => {
                self.asked = Some(text.clone());
            }
            Event::Ask {
                id,
                call,
                tool,
                input,
                title,
                detail,
                ..
            } => {
                self.pending.insert(
                    id.clone(),
                    Pending {
                        tool: tool.clone(),
                        call: call.clone(),
                        input: input.clone(),
                        title: title.clone(),
                        detail: detail.clone(),
                    },
                );
            }
            Event::Answered { id, .. } => {
                self.pending.remove(id);
            }
            Event::Exited { .. } => {
                self.busy = None;
                // Nothing can be approved for a process that has gone, and a question left
                // standing would keep saying the session is waiting.
                self.pending.clear();
                // Nor can a command be written to it, so no chip may promise one is about
                // to run.
                self.held.clear();
            }
            _ => {}
        }
        // Remember what each call is, so a detached one can be named — `State::called`.
        if let Event::Tool { id, name, does } = &event {
            if self.called.len() >= CALLED_RING {
                self.called.pop_front();
            }
            self.called
                .push_back((id.clone(), crate::protocol::called(name, does)));
        }
        // Work left running is decided where the events are read — [`protocol::running`].
        match protocol::running(&event) {
            protocol::Running::Began { tool, task } => {
                // Unnamed rather than absent when the ring has rolled past it: that it is
                // running is the fact worth keeping.
                let mut named = self
                    .called
                    .iter()
                    .find(|(id, _)| *id == tool)
                    .map(|(_, called)| called.clone())
                    .unwrap_or_else(|| crate::protocol::Called {
                        tool: String::from("tool"),
                        label: None,
                        task: None,
                    });
                named.task = task;
                self.background.insert(tool, named);
            }
            protocol::Running::Ended(named) => forget(&mut self.background, &named),
            // By the task, because that is the only name a kill gives.
            protocol::Running::Killed(task) => {
                forget(&mut self.background, &protocol::Named::Task(task));
            }
            protocol::Running::Gone => self.background.clear(),
            protocol::Running::Quiet => {}
        }
        in_flight(self, &event, now);
        self.issued += 1;
        let stamped = Stamped {
            seq: self.issued,
            at,
            event,
        };
        if self.log.len() >= SCROLLBACK {
            self.log.pop_front();
        }
        self.log.push_back(stamped.clone());
        stamped
    }
}
