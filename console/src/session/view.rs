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
            effort: state.effort.clone(),
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

    /// Questions this session still waits on, which no transcript records.
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

    /// How long this session has failed to read what was written to it, in
    /// milliseconds: a message in flight, between turns, for long enough. Blind to a
    /// session that goes deaf mid-turn. See [`crate::deaf`].
    pub fn deaf(&self) -> Option<i64> {
        deaf_for(&self.state.lock(), now())
    }

    /// What this session was last told it may do without asking. See [`Summary::mode`].
    pub fn mode(&self) -> Option<crate::modes::Mode> {
        self.state.lock().mode.clone()
    }

    /// What was written and never read, oldest first — resent by
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

    /// What a client holding everything through `after` still needs — or a fresh
    /// start, never an approximation. See [resumable].
    pub fn since(&self, after: Option<u64>) -> Backlog {
        let state = self.state.lock();
        // An empty log holds from the next number.
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

    /// The last sequence number issued, for a client seeded from the transcript.
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

    /// Whether a turn is running — [`Summary::working`].
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
            // Filled by the roster.
            touched: None,
            bytes: None,
            alive: state.alive,
            model: state.model.clone(),
            effort: state.effort.clone(),
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
            // Filled by the roster.
            name: None,
            peer_name: None,
            waiting: state.pending.len(),
            unread: state.unread.len(),
            deaf: deaf_for(&state, now()).map(|ms| (ms / 1000) as u64),
            signed_out: state.signed_out.map(|ms| ms as u64),
            held: state.held.iter().map(|held| held.text.clone()).collect(),
        }
    }
}
