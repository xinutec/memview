//! Every message this console has handed a session, by id: the one record of
//! where each stands — held for the turn's end, handed to the CLI, waiting in its
//! queue, read, taken back, or dropped with the process.
//!
//! Its steps are the CLI's own word (`command_lifecycle`, and the echo of a
//! message read), all measured on 2.1.289: a message is queued, then started
//! when a turn takes it in; `cancel_async_message` takes it out until then, and a
//! cancel that comes before its message takes it out when it arrives; a stop lets
//! the CLI read its queue first, a kill does not. Carried across an upgrade with
//! the session, so nothing here is guessed afresh.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::protocol::Lifecycle;

/// How long a message is remembered once it is settled, so a late copy of it is
/// answered rather than written: a phone gives up on a send long before this.
const KEPT_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Where one message stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// A slash command held here for the turn's end.
    Held,
    /// Written to the CLI, which has not said it queued it.
    Written,
    /// In the CLI's queue: it can be taken back.
    Queued,
    /// The session has read it. `echoed` once its echo has been shown, so a
    /// second echo — of a retry the CLI ignored — is not shown again.
    Read { at: i64, echoed: bool },
    /// Taken back before the session read it; its words went back once.
    Withdrawn,
    /// The process ended with it unread, and nothing will read it now.
    Dropped,
}

impl Step {
    /// Not read, and still able to be: held, written or queued.
    pub fn waiting(self) -> bool {
        matches!(self, Step::Held | Step::Written | Step::Queued)
    }

    fn unread(self) -> bool {
        matches!(self, Step::Written | Step::Queued)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub text: String,
    /// The names its pictures were kept under, in order.
    #[serde(default)]
    pub pictures: Vec<String>,
    /// When it was handed over, in epoch milliseconds.
    pub at: i64,
    /// Arrival order, which `at` cannot break ties in.
    order: u64,
    pub step: Step,
}

/// A message not yet read or settled, as the phone is shown it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Waiting {
    pub id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub pictures: Vec<String>,
    pub at: i64,
    pub stands: Stands,
}

/// How a waiting message stands, in the words a person needs.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Stands {
    /// Runs when the turn ends.
    Held,
    /// Waiting to be read.
    Unread,
    /// Will never be read: its session ended.
    Dropped,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Messages {
    by_id: BTreeMap<String, Message>,
    next: u64,
}

impl Messages {
    pub fn get(&self, id: &str) -> Option<&Message> {
        self.by_id.get(id)
    }

    /// Record a message handed over at `now`, forgetting what has long settled.
    pub fn add(&mut self, id: &str, text: &str, pictures: Vec<String>, step: Step, now: i64) {
        self.by_id
            .retain(|_, message| message.step.waiting() || now - message.at < KEPT_MS);
        self.next += 1;
        self.by_id.insert(
            id.to_string(),
            Message {
                text: text.to_string(),
                pictures,
                at: now,
                order: self.next,
                step,
            },
        );
    }

    pub fn set(&mut self, id: &str, step: Step) {
        if let Some(message) = self.by_id.get_mut(id) {
            message.step = step;
        }
    }

    /// The CLI's word on a message. Read stays read: a turn aborted after taking a
    /// message in reports it cancelled, and it was seen all the same.
    pub fn stepped(&mut self, id: &str, step: Lifecycle, now: i64) {
        let Some(message) = self.by_id.get_mut(id) else {
            return;
        };
        message.step = match (message.step, step) {
            (read @ Step::Read { .. }, _) => read,
            (Step::Written, Lifecycle::Queued) => Step::Queued,
            (_, Lifecycle::Started | Lifecycle::Completed) => Step::Read {
                at: now,
                echoed: false,
            },
            (_, Lifecycle::Cancelled) => Step::Withdrawn,
            (Step::Held | Step::Written | Step::Queued, Lifecycle::Dropped) => Step::Dropped,
            (step, _) => step,
        };
    }

    /// The echo of a message read, by its id: true when it was echoed already —
    /// a retry the CLI ignored — and so not to be shown twice.
    pub fn echoed(&mut self, id: &str, now: i64) -> bool {
        let Some(message) = self.by_id.get_mut(id) else {
            return false;
        };
        match message.step {
            Step::Read { echoed: true, .. } => true,
            Step::Read { at, .. } => {
                message.step = Step::Read { at, echoed: true };
                false
            }
            _ => {
                message.step = Step::Read {
                    at: now,
                    echoed: true,
                };
                false
            }
        }
    }

    /// The echo of a message with no id: the oldest unread one saying these words.
    pub fn echoed_words(&mut self, text: &str, now: i64) {
        let oldest = self
            .by_id
            .iter()
            .filter(|(_, message)| message.step.unread() && message.text == text)
            .min_by_key(|(_, message)| message.order)
            .map(|(id, _)| id.clone());
        if let Some(id) = oldest {
            self.echoed(&id, now);
        }
    }

    /// The oldest command held for the turn's end, taken off the hold.
    pub fn next_held(&self) -> Option<(String, Message)> {
        self.by_id
            .iter()
            .filter(|(_, message)| message.step == Step::Held)
            .min_by_key(|(_, message)| message.order)
            .map(|(id, message)| (id.clone(), message.clone()))
    }

    /// When the oldest message written and not yet read was handed over.
    pub fn oldest_unread(&self) -> Option<i64> {
        self.by_id
            .values()
            .filter(|message| message.step.unread())
            .map(|message| message.at)
            .min()
    }

    /// Messages written and not yet read, oldest first, for a new process.
    pub fn unread(&self) -> Vec<(String, Message)> {
        let mut unread: Vec<_> = self
            .by_id
            .iter()
            .filter(|(_, message)| message.step.unread())
            .map(|(id, message)| (id.clone(), message.clone()))
            .collect();
        unread.sort_by_key(|(_, message)| message.order);
        unread
    }

    /// The process is gone: nothing it held or had queued will be read.
    pub fn drop_waiting(&mut self) {
        for message in self.by_id.values_mut() {
            if message.step.waiting() {
                message.step = Step::Dropped;
            }
        }
    }

    /// What the phone is shown: every message not yet read, and those dropped
    /// until taken back, oldest first.
    pub fn waiting(&self) -> Vec<Waiting> {
        let mut shown: Vec<_> = self
            .by_id
            .iter()
            .filter_map(|(id, message)| {
                let stands = match message.step {
                    Step::Held => Stands::Held,
                    Step::Written | Step::Queued => Stands::Unread,
                    Step::Dropped => Stands::Dropped,
                    Step::Read { .. } | Step::Withdrawn => return None,
                };
                Some((
                    message.order,
                    Waiting {
                        id: id.clone(),
                        text: message.text.clone(),
                        pictures: message.pictures.clone(),
                        at: message.at,
                        stands,
                    },
                ))
            })
            .collect();
        shown.sort_by_key(|(order, _)| *order);
        shown.into_iter().map(|(_, waiting)| waiting).collect()
    }
}
