//! What a session answers when asked: its summary, its history, what it is
//! waiting on.

use std::time::UNIX_EPOCH;

use tokio::sync::broadcast;

use super::state::deaf_for;
use super::{Backlog, Fds, Pending, Session, Stamped, Summary, Tally, now, resumable};
use crate::protocol::Event;

impl Session {
    /// What this session has counted, for an upgrade to hand on. See [`Tally`].
    pub fn tally(&self) -> Tally {
        let state = self.state.lock();
        Tally {
            started: self
                .started
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            model: state.model.clone(),
            cost_usd: state.cost_usd,
            window: state.window,
            limit: state.limit.clone(),
            spent: state.spent.clone(),
            mode: state.mode.clone(),
            asked: state.asked.clone(),
            busy: state.busy.clone(),
            pending: state.pending.clone(),
            background: state.background.clone(),
            counted: state.counted,
        }
    }

    /// Questions this session is still waiting on. A cold reader has to be offered
    /// these again: an `Ask` is a control request no transcript records, so a seed
    /// cannot contain one — `crate::api::cold` needs this as [`super::Session::adopt`] does.
    pub fn asking(&self) -> Vec<(String, Pending)> {
        self.state
            .lock()
            .pending
            .iter()
            .map(|(id, question)| (id.clone(), question.clone()))
            .collect()
    }

    /// The pipes to this session's process, for an upgrade to hand on.
    pub fn fds(&self) -> Fds {
        self.fds
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// When this session's process last said anything. See [`Self::heard`].
    pub fn last_heard(&self) -> i64 {
        self.state.lock().heard
    }

    /// How long this session has been failing to read what was written to it, in
    /// milliseconds — `None` for one that is merely busy, or quiet. See [`crate::deaf`].
    ///
    /// Three things at once: a message is in flight; the session is between turns
    /// ([`super::State::idle_since`]) — one mid tool call parks input on purpose; and long
    /// enough, [`super::state::DEAF_AFTER_MS`] or [`super::state::DEAF_AFTER_COMPACT_MS`]. The clock starts at
    /// whichever came second. It cannot see a session that goes deaf mid-turn.
    pub fn deaf(&self) -> Option<i64> {
        deaf_for(&self.state.lock(), now())
    }

    /// What this session was last told it may do without asking. See [`Summary::mode`].
    pub fn mode(&self) -> Option<crate::modes::Mode> {
        self.state.lock().mode.clone()
    }

    /// What was written to this session and never read, oldest first — the other
    /// half of the cure, since a restart loses the old pipe. See
    /// [`crate::roster::Roster::revive`].
    pub fn unread(&self) -> Vec<String> {
        self.state
            .lock()
            .unread
            .iter()
            .map(|held| held.text.clone())
            .collect()
    }

    /// The transcript so far, unnumbered — for asking what a session has done.
    pub fn history(&self) -> Vec<Event> {
        self.state
            .lock()
            .log
            .iter()
            .map(|stamped| stamped.event.clone())
            .collect()
    }

    /// What a client that says it holds everything through `after` still needs.
    /// Resuming is refused rather than approximated — see [resumable].
    pub fn since(&self, after: Option<u64>) -> Backlog {
        let state = self.state.lock();
        // With an empty log nothing is held, so the earliest number still honourable
        // is the next one to be issued.
        let held_from = state
            .log
            .front()
            .map_or(state.issued + 1, |first| first.seq);
        if let Some(after) = after
            && resumable(after, held_from, state.issued)
        {
            let events: Vec<Stamped> = state
                .log
                .iter()
                .filter(|stamped| stamped.seq > after)
                .cloned()
                .collect();
            let through = events.last().map_or(after, |last| last.seq);
            return Backlog {
                resumed: true,
                events,
                through,
            };
        }
        let events: Vec<Stamped> = state.log.iter().cloned().collect();
        let through = events.last().map_or(0, |last| last.seq);
        Backlog {
            resumed: false,
            events,
            through,
        }
    }

    /// The last sequence number this session has issued, for a client seeded from
    /// the transcript — see [`crate::api`]. Every event at or below it has been
    /// written, so nothing after it is sent twice.
    pub fn issued(&self) -> u64 {
        self.state.lock().issued
    }

    pub fn listen(&self) -> broadcast::Receiver<Stamped> {
        self.tx.subscribe()
    }

    /// What the child said on stderr, for when it will not start.
    pub fn trouble(&self) -> String {
        self.state.lock().stderr.clone()
    }

    pub fn alive(&self) -> bool {
        self.state.lock().alive
    }

    /// Whether a turn is running right now — [`Summary::working`], and not
    /// [`Summary::busy`], which cannot answer this.
    pub fn working(&self) -> bool {
        self.state.lock().working
    }

    pub fn summary(&self) -> Summary {
        let state = self.state.lock();
        Summary {
            id: self.id.clone(),
            dir: self.dir.display().to_string(),
            started: self
                .started
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            // Left for the roster, which reads the transcript once per listing.
            touched: None,
            bytes: None,
            alive: state.alive,
            model: state.model.clone(),
            busy: state.busy.clone(),
            working: state.working,
            interactions: state.counted.interactions,
            mode: state.mode.clone(),
            mode_refused: state.mode_refused.clone(),
            cost_usd: state.cost_usd,
            limit: state.limit.clone(),
            context: state.context,
            window: state.window,
            background: state.background.len(),
            running: state.background.values().cloned().collect(),
            asked: state.asked.clone(),
            // Filled in by the roster, which knows where the transcripts are.
            name: None,
            peer_name: None,
            waiting: state.pending.len(),
            unread: state.unread.len(),
            deaf: deaf_for(&state, now()).map(|ms| (ms / 1000) as u64),
            held: state.held.iter().cloned().collect(),
        }
    }
}
