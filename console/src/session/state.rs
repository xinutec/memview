//! The mutable half of a session and how each event changes it.

use std::collections::{BTreeMap, VecDeque};

use super::{Heard, ResetsAt, Seen};
use crate::protocol::{self, Event};

/// How much transcript one session keeps in memory: a scrollback, not an archive.
const SCROLLBACK: usize = 5000;

/// How many recent tool calls are remembered so a detached one can be named.
/// Small: the result that reveals a detach follows its call at once.
const CALLED_RING: usize = 32;

/// An event and its place in the session's order, from 1, assigned under the
/// log's lock. A reconnecting client says what it has and is sent the rest.
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
/// missed from a log holding `held_from..=issued`. `after + 1`: the next event must
/// be reachable, so a caught-up client resumes against an empty log.
pub fn resumable(after: u64, held_from: u64, issued: u64) -> bool {
    after + 1 >= held_from && after <= issued
}

/// A question the session waits on. The arguments are kept because an allow
/// echoes them back; the tool, because only [`protocol::QUESTION_TOOL`]'s may be edited.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Pending {
    pub tool: String,
    /// The `tool_use` id asked about, kept across an upgrade for the re-seeded question.
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
    /// The last sequence number issued — not the log's length, which drops its front.
    pub(super) issued: u64,
    /// Questions the session is blocked on, by control-request id.
    pub(super) pending: BTreeMap<String, Pending>,
    pub(super) alive: bool,
    /// When the kill armed by [`Session::stop`] falls due, in epoch milliseconds.
    pub(super) stopping: Option<i64>,
    pub(super) model: Option<String>,
    /// See [`Summary::effort`].
    pub(super) effort: Option<crate::protocol::Effort>,
    /// What this session can be told — [`crate::protocol::initialize`]'s answer.
    pub(super) commands: Option<std::sync::Arc<crate::protocol::Commands>>,
    pub(super) busy: Option<String>,
    /// See [`Summary::interactions`] and [`crate::past::counted`].
    pub(super) counted: crate::past::Counted,
    /// See [`Summary::mode`]. Written when asked for, corrected by [`Session::settle_mode`].
    pub(super) mode: Option<crate::modes::Mode>,
    /// The mode before an unanswered change, for a refusal to put back.
    pub(super) restore: Option<crate::modes::Mode>,
    /// See [`Summary::mode_refused`].
    pub(super) mode_refused: Option<String>,
    pub(super) cost_usd: f64,
    /// See [`Summary::context`].
    pub(super) context: Option<u64>,
    pub(super) window: Option<u64>,
    /// See [`Summary::limit`].
    pub(super) limit: Option<crate::protocol::Allowance>,
    /// What the API last said about each rate-limit window. See [`Tally::spent`].
    pub(super) spent: BTreeMap<String, Seen>,
    /// Background calls not yet ended, by call id (what a notification names),
    /// carrying the task id (what a kill names).
    pub(super) background: std::collections::BTreeMap<String, crate::protocol::Called>,
    /// The latest end each piece of background work declared, by call id, in epoch
    /// milliseconds. Only decides when an entry is overdue.
    pub(super) deadlines: std::collections::BTreeMap<String, i64>,
    /// The last few tool calls, by call id, so a background one can be named. A
    /// ring, for a process that runs for days.
    pub(super) called: std::collections::VecDeque<(String, crate::protocol::Called)>,
    /// When the process last wrote a line, in epoch milliseconds — [`Session::heard`].
    pub(super) heard: i64,
    /// Whether the transcript was searched for the origin — which `asked: None`
    /// cannot say. Only a session with no transcript is named by its next prompt.
    pub(super) origin_read: bool,
    pub(super) asked: Option<String>,
    pub(super) stderr: String,
    /// Messages written to stdin and not echoed back, oldest first. See [`Session::deaf`].
    pub(super) unread: VecDeque<Unread>,
    /// Whether the session has spoken since its last turn ended — [`Summary::working`].
    /// Not `idle_since.is_none()`, which a session that has said nothing also is.
    pub(super) working: bool,
    /// When the last turn ended, in epoch milliseconds; `None` while working.
    pub(super) idle_since: Option<i64>,
    /// When the API last answered this process, in epoch milliseconds — the age of
    /// the usage it would repeat. See [`crate::usage::echo_dated`].
    pub(super) answered: Option<i64>,
    /// Whether this episode of deafness has been announced.
    pub(super) announced_deaf: bool,
    /// When the oldest decision not yet acted on was written, in epoch milliseconds.
    pub(super) decided: Option<i64>,
    /// When the sign-in was refused, in epoch milliseconds, until a request is answered.
    pub(super) signed_out: Option<i64>,
    /// Slash commands written mid-turn, oldest first, which the CLI would hand to the
    /// model as words. Held here, not in a client that may be put away. See
    /// [`Session::release_held`].
    pub(super) held: VecDeque<Held>,
    /// The ids the CLI has echoed back to this process, with when: a second echo of
    /// one is a retry it ignored, not a second message. See [`crate::sent`].
    pub(super) echoed: std::collections::BTreeMap<String, i64>,
    /// A `/compact` is outstanding: the one long silence that is not a fault.
    pub(super) compacting: bool,
}

/// How long a message may sit unread between turns before the session is called
/// deaf. Real waits are seconds; a deaf session is silent for tens of minutes.
pub(super) const DEAF_AFTER_MS: i64 = 90_000;

/// The same while a compaction, which takes minutes, is outstanding.
pub(super) const DEAF_AFTER_COMPACT_MS: i64 = 15 * 60_000;

impl State {
    /// Work more than `grace` past the deadline it declared, taken off the count
    /// and returned. A deadline whose work already ended is dropped with it.
    pub(super) fn overdue(&mut self, now: i64, grace: i64) -> Vec<crate::protocol::Called> {
        let due: Vec<String> = self
            .deadlines
            .iter()
            .filter(|(_, at)| **at + grace < now)
            .map(|(id, _)| id.clone())
            .collect();
        due.into_iter()
            .filter_map(|id| {
                self.deadlines.remove(&id);
                self.background.remove(&id)
            })
            .collect()
    }
}

/// Drop background work from the count by whichever name ended it.
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

/// Everything [`Session::deaf`] decides on, kept in one place: the verdict is a
/// conjunction, and a field updated in only some arms fails silently.
pub(super) fn in_flight(state: &mut State, event: &Event, now: i64) {
    match event {
        // The read receipt. Oldest match first: stdin is a queue.
        Event::Prompt { text, id } => {
            // By id when it has one: two messages may say the same words.
            let found = match id {
                Some(id) => state
                    .unread
                    .iter()
                    .position(|held| held.id.as_deref() == Some(id.as_str())),
                None => None,
            }
            .or_else(|| state.unread.iter().position(|held| &held.text == text));
            if let Some(at) = found {
                state.unread.remove(at);
            }
            // It read: the next episode is worth announcing.
            state.announced_deaf = false;
        }
        // A turn ended, so from here the session owes us a read.
        Event::Turn { .. } => state.idle_since = Some(now),
        // Nothing runs yet — or a seeded file ending mid-turn reads as a running turn.
        Event::Started { .. } | Event::Joined { .. } => {
            state.idle_since = Some(now);
            state.answered = None;
        }
        _ => {}
    }
    // What the API sent back: each carries fresh rate-limit headers.
    if matches!(
        event,
        Event::Text { .. } | Event::Tool { .. } | Event::Context { .. } | Event::Turn { .. }
    ) {
        state.answered = Some(now);
    }
    state.working = working_after(state.working, event);
    match event {
        Event::Command { text } if text.starts_with("/compact") => state.compacting = true,
        // A decision gets its own clock: a blocked session is mid-turn, so the
        // message test cannot fire. Read while the question is still pending.
        Event::Answered { id, .. } if leaves_a_decision(state.pending.get(id)) => {
            state.decided = state.decided.or(Some(now));
        }
        _ => {}
    }
    // Anything it says of its own accord is work. Not `Busy`, which is announced
    // only on change; and a `Turn` must not clear the `idle_since` it just set.
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

/// Whether a turn is running after `event`: set by the session speaking, cleared
/// by the turn ending — and by `Started` and `Joined`, for a file ending mid-turn.
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
        // A status, a decision, a command: no change.
        _ => was,
    }
}

/// [`Session::deaf`], against state the caller already holds.
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

/// The verdict as arithmetic, testable without waiting. Two ways to be waiting —
/// an unread message once the turn has ended (`idle_since`, `oldest`), or a
/// decision not acted on (`decided`) — and the longer wait is reported.
pub fn deaf_after(
    idle_since: Option<i64>,
    oldest: Option<i64>,
    decided: Option<i64>,
    compacting: bool,
    now: i64,
) -> Option<i64> {
    // From whichever came second: only then could the session have read it.
    let unread_since = idle_since.zip(oldest).map(|(idle, at)| idle.max(at));
    // The longer wait, so a short one cannot hide it.
    let since = [unread_since, decided].into_iter().flatten().min()?;
    let allowed = if compacting {
        DEAF_AFTER_COMPACT_MS
    } else {
        DEAF_AFTER_MS
    };
    let waited = now - since;
    (waited >= allowed).then_some(waited)
}

/// Whether a decision has been acted on: an allowed command runs as a child started
/// after the answer. One started before is other work.
pub fn taken_up(decided: i64, children_started: &[i64]) -> bool {
    children_started.iter().any(|&started| started >= decided)
}

/// Whether an answer leaves the session something to act on. A root-password
/// question does not: the console runs that command itself.
pub fn leaves_a_decision(answered: Option<&Pending>) -> bool {
    answered.is_none_or(|asked| asked.tool != crate::sudo::TOOL)
}

/// A message written to stdin and not yet replayed by the CLI. Commands are never
/// replayed, so never held.
#[derive(Debug, Clone)]
pub(super) struct Unread {
    pub(super) text: String,
    /// When it was written, in epoch milliseconds.
    pub(super) at: i64,
    /// The client's id for it, when it gave one.
    pub(super) id: Option<String>,
}

/// A slash command held for the turn's end, with the client's id for it.
#[derive(Debug, Clone)]
pub(super) struct Held {
    pub(super) text: String,
    pub(super) id: Option<String>,
}

impl State {
    /// Fold one event in and number it. `now` dates an event that carries no time —
    /// which marks it an echo, not a measurement.
    pub(super) fn take(&mut self, event: Event, at: Option<i64>, now: i64) -> Stamped {
        // First, while an answered question is still pending — [`in_flight`].
        in_flight(self, &event, now);
        match &event {
            Event::Started { model, .. } => self.model = Some(model.clone()),
            Event::Busy { status } => self.busy = Some(status.clone()),
            // The window, which only the result line declares.
            Event::Context { tokens } => self.context = Some(*tokens),
            // The count was of a conversation the summary replaced; the next message
            // brings a real one.
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
                // Not `num_turns`, which counts messages, not exchanges.
            }
            Event::Limit {
                window,
                status,
                resets_at,
                utilization,
            } => {
                self.limit = Some(status.clone());
                // One window per event: collected, not replaced.
                if let Some(spent) = utilization {
                    // Measured only when the event carries its own time.
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
            Event::Prompt { text, .. } if self.asked.is_none() && !self.origin_read => {
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
                // A gone process can be asked nothing and sent nothing.
                self.pending.clear();
                self.held.clear();
            }
            _ => {}
        }
        // Signed out until a real request is answered — one with a prompt, which the
        // synthetic refusal's zero usage lacks.
        match &event {
            Event::SignedOut => self.signed_out = Some(at.unwrap_or(now)),
            Event::Context { tokens } if *tokens > 0 => self.signed_out = None,
            Event::Tool { .. } => self.signed_out = None,
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
            protocol::Running::Began {
                tool,
                task,
                expires_in,
            } => {
                // From the event's own time, so replayed work falls due when it did.
                if let Some(ms) = expires_in {
                    self.deadlines.insert(tool.clone(), at.unwrap_or(now) + ms);
                }
                // Unnamed if the ring has rolled past it: that it runs is what matters.
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
            protocol::Running::Gone => {
                self.background.clear();
                self.deadlines.clear();
            }
            protocol::Running::Quiet => {}
        }
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
