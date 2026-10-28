//! What the console writes to a session: messages, pictures, decisions and
//! control requests, and the replies it reads back.

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use super::messages::{Message, Step};
use crate::sent::{Refusal, Why};

/// How long a withdrawal waits for the CLI: it answers from its queue, at once.
const WITHDRAW_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

use super::{ResetsAt, Seen, Session, now};
use crate::protocol::{self, Event};

impl Session {
    /// Write one line to the session and flush it; `doing` names the write in the
    /// error. The lock is held for the line alone.
    async fn write_line(&self, line: &str, doing: &str) -> Result<()> {
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .with_context(|| doing.to_string())?;
        stdin
            .flush()
            .await
            .with_context(|| format!("flushing: {doing}"))?;
        Ok(())
    }

    /// Send a message under an id of its own — for the console itself and tests;
    /// a client gives its own, through [`Self::offer`].
    pub async fn send(&self, text: &str) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        let line = protocol::prompt(text, Some(&id));
        self.offer(&id, text, Vec::new(), line).await.map(|_| ())
    }

    /// Hand the session a message under the client's id, written as `line`: once,
    /// however often it is offered. A slash command sent mid-turn is held for the
    /// turn's end, since the CLI would hand it to the model as words.
    pub async fn offer(
        &self,
        id: &str,
        text: &str,
        pictures: Vec<String>,
        line: String,
    ) -> Result<Offered> {
        let _deciding = self.deciding.lock().await;
        if let Some(message) = self.state.lock().messages.get(id) {
            return Ok(match message.step {
                Step::Withdrawn => Offered::Refused(Refusal {
                    why: Why::Withdrawn,
                    at: None,
                }),
                _ => Offered::Already,
            });
        }
        // Written by a process before this one, which the transcript remembers.
        if let Some(at) = self.recorded(id) {
            let read = Step::Read { at, echoed: true };
            self.state
                .lock()
                .messages
                .add(id, text, pictures, read, now());
            return Ok(Offered::Already);
        }
        let command = protocol::is_command(text);
        if command && self.state.lock().working {
            self.state
                .lock()
                .messages
                .add(id, text, pictures, Step::Held, now());
            return Ok(Offered::Wrote);
        }
        self.write_line(&line, "writing to the session").await?;
        let mut state = self.state.lock();
        if state.asked.is_none() && !state.origin_read {
            // Named by the words, not the base64.
            state.asked = Some(match (text.trim(), pictures.len()) {
                ("", 1) => "an image".to_string(),
                ("", many) if many > 1 => format!("{many} images"),
                (words, _) => words.to_string(),
            });
        }
        // A command is never echoed, and runs as a command: said once it is written.
        let step = if command {
            Step::Read {
                at: now(),
                echoed: true,
            }
        } else {
            Step::Written
        };
        state.messages.add(id, text, pictures, step, now());
        drop(state);
        // Never echoed, so said here; the CLI may not run it for minutes.
        if command {
            self.push(Event::Command {
                text: text.to_string(),
            });
        }
        Ok(Offered::Wrote)
    }

    /// Send the commands held for this turn's end, oldest first. A failed write
    /// stops it, leaving the rest held.
    pub async fn release_held(&self) -> Result<()> {
        let _deciding = self.deciding.lock().await;
        loop {
            let next = self.state.lock().messages.next_held();
            let Some((id, held)) = next else {
                return Ok(());
            };
            self.write_line(
                &protocol::prompt(&held.text, Some(&id)),
                "writing to the session",
            )
            .await?;
            let said = Step::Read {
                at: now(),
                echoed: true,
            };
            self.state.lock().messages.set(&id, said);
            self.push(Event::Command { text: held.text });
        }
    }

    /// Take a message back while the session has not read it. Its words go into
    /// its conversation's draft, once, however often this is asked.
    pub async fn withdraw(&self, id: &str) -> Result<Withdrawal> {
        let _deciding = self.deciding.lock().await;
        let known = self.state.lock().messages.get(id).cloned();
        let Some(message) = known else {
            if let Some(at) = self.recorded(id) {
                return Ok(Withdrawal::Refused(read_at(at)));
            }
            // Never arrived: remembered as taken back, so a late copy is refused.
            self.state
                .lock()
                .messages
                .add(id, "", Vec::new(), Step::Withdrawn, now());
            return Ok(Withdrawal::Back(None));
        };
        match message.step {
            Step::Read { at, .. } => return Ok(Withdrawal::Refused(read_at(at))),
            // Its words went to the draft when it was taken back; asked again —
            // the answer lost on the way, or a second tap — it is answered again.
            Step::Withdrawn => return Ok(Withdrawal::Back(Some(message))),
            Step::Held | Step::Dropped => {}
            Step::Written | Step::Queued => {
                // Not in the queue at that moment: read, or not yet arrived — and a
                // cancel that comes first takes the message out the moment it does
                // (measured, at twenty seconds' wait). Which it is was said before
                // this answer: the CLI writes `started` when a turn takes a message,
                // before it answers a cancel that no longer finds it, and lines are
                // read in order.
                let cancelled = self.cancel_in_cli(id).await?;
                if !cancelled
                    && let Some(Step::Read { at, .. }) =
                        self.state.lock().messages.get(id).map(|it| it.step)
                {
                    return Ok(Withdrawal::Refused(read_at(at)));
                }
            }
        }
        let taken = self.state.lock().messages.withdraw(id);
        self.give_back(taken);
        Ok(Withdrawal::Back(Some(message)))
    }

    /// The words of a message just taken back, into its conversation's draft.
    fn give_back(&self, taken: Option<Message>) {
        if let Some(message) = taken.filter(|it| !it.text.is_empty()) {
            self.drafts.give_back(&self.id, &message.text, now() as u64);
        }
    }

    /// Ask the CLI to take a message out of its queue: true when it was there.
    async fn cancel_in_cli(&self, id: &str) -> Result<bool> {
        let request = format!("{}{id}", protocol::WITHDRAW);
        let (answered, answer) = tokio::sync::oneshot::channel();
        self.withdrawals.lock().insert(request.clone(), answered);
        let asked = self
            .write_line(&protocol::withdraw(&request, id), "taking a message back")
            .await;
        if let Err(error) = asked {
            self.withdrawals.lock().remove(&request);
            return Err(error);
        }
        match tokio::time::timeout(WITHDRAW_PATIENCE, answer).await {
            Ok(Ok(cancelled)) => Ok(cancelled),
            Ok(Err(_)) => anyhow::bail!("the session ended before it answered"),
            Err(_) => {
                self.withdrawals.lock().remove(&request);
                anyhow::bail!("the session did not answer in time")
            }
        }
    }

    /// Hand a withdrawal's answer to whoever is waiting for it.
    pub(super) fn answer_withdrawal(&self, request: &str, cancelled: bool) {
        if let Some(waiting) = self.withdrawals.lock().remove(request) {
            let _ = waiting.send(cancelled);
        }
    }

    /// The CLI's word on where a message stands in its queue.
    pub(super) fn stepped(&self, id: &str, step: protocol::Lifecycle) {
        let taken = self.state.lock().messages.stepped(id, step, now());
        self.give_back(taken);
    }

    /// Whether `event` is the echo of a message already echoed: the CLI echoes a
    /// retried id but records and answers it once. The first echo marks it read.
    pub fn echoed_again(&self, event: &Event) -> bool {
        let mut state = self.state.lock();
        match event {
            Event::Prompt { id: Some(id), .. } | Event::Shown { id: Some(id), .. } => {
                state.messages.echoed(id, now())
            }
            Event::Prompt { text, id: None } => {
                state.messages.echoed_words(text, now());
                false
            }
            _ => false,
        }
    }

    /// Whether this message was handed over already, here or before this process.
    pub fn knows(&self, id: &str) -> bool {
        self.state.lock().messages.get(id).is_some() || self.recorded(id).is_some()
    }

    /// When this session's transcript recorded the message `id`, if it did.
    fn recorded(&self, id: &str) -> Option<i64> {
        crate::past::transcript_of(&crate::past::projects_root(), &self.id)
            .and_then(|transcript| crate::sent::recorded_at(&transcript, id))
    }

    /// Answer a question the session is blocked on. An unknown id is an error — two
    /// people answering one session. `reply` only for [`protocol::QUESTION_TOOL`]:
    /// approving a call must not rewrite it.
    pub async fn decide(
        &self,
        id: &str,
        allowed: bool,
        why: &str,
        reply: Option<&protocol::Reply>,
    ) -> Result<()> {
        let pending = {
            let state = self.state.lock();
            state
                .pending
                .get(id)
                .cloned()
                .context("that question is not open — it may already have been answered")?
        };
        if reply.is_some() && pending.tool != protocol::QUESTION_TOOL {
            anyhow::bail!(
                "answers were sent for {}, which does not ask questions",
                pending.tool
            );
        }
        let line = protocol::decision(id, allowed, &pending.input, why, reply);
        self.write_line(&line, "answering the session").await?;
        self.push(Event::Answered {
            id: id.to_string(),
            allowed,
            reply: reply.cloned(),
        });
        Ok(())
    }

    /// Rename the conversation by control request — `/rename` mid-turn reaches the
    /// model as words. The title only: peers see it after the next resume
    /// ([`super::Spawn::name`]).
    pub async fn rename(&self, title: &str) -> Result<()> {
        let line = protocol::rename(&format!("rename-{}", self.id), title);
        self.write_line(&line, "renaming the session").await?;
        Ok(())
    }

    /// Change what this session may do without asking. Recorded once written,
    /// before the CLI answers.
    pub async fn set_mode(&self, mode: &crate::modes::Mode) -> Result<()> {
        let line = protocol::set_mode(&format!("set-mode-{}", self.id), mode);
        self.write_line(&line, "asking the session to change mode")
            .await?;
        let mut state = self.state.lock();
        // For a refusal to put back — the real mode, not an outstanding guess.
        if state.restore.is_none() {
            state.restore = state.mode.clone();
        }
        // A new attempt clears the old refusal.
        state.mode_refused = None;
        state.mode = Some(mode.clone());
        Ok(())
    }

    /// Take the CLI at its word about its mode, correcting [`super::Session::set_mode`].
    pub(super) fn settle_mode(&self, reply: protocol::ModeReply) {
        let mut state = self.state.lock();
        match reply {
            protocol::ModeReply::Now(mode) => {
                state.mode = Some(mode);
                state.mode_refused = None;
            }
            protocol::ModeReply::Refused(why) => {
                tracing::info!("{}: the mode change was refused — {why}", self.id);
                // Back to what it was, if this console made the change.
                if let Some(was) = state.restore.clone() {
                    state.mode = Some(was);
                }
                state.mode_refused = Some(why);
            }
        }
        state.restore = None;
    }

    /// Keep what the CLI answered, per window.
    pub(super) fn record_usage(&self, reply: protocol::UsageReply) {
        let mut state = self.state.lock();
        let (at, measured) = crate::usage::echo_dated(reply.fetched, state.answered, now());
        let heard: Vec<(String, Seen)> = reply
            .windows
            .into_iter()
            .map(|(window, utilization, resets_at)| {
                (
                    window,
                    Seen {
                        utilization,
                        resets_at: resets_at.map(ResetsAt),
                        at,
                        measured,
                    },
                )
            })
            .collect();
        // An echo that disagrees and is not believed: logged.
        for (window, seen) in &heard {
            if let Some(held) = state.spent.get(window)
                && held.utilization != seen.utilization
                && !crate::usage::fresher(held, seen)
            {
                tracing::info!(
                    "{}: kept {window} at {:.0}% over an echo of {:.0}%",
                    self.id,
                    held.utilization * 100.0,
                    seen.utilization * 100.0,
                );
            }
        }
        // An echo must not overwrite a fresh measurement.
        crate::usage::remember(&mut state.spent, heard);
    }

    /// Ask what this session can be told — [`protocol::initialize`]; the answer
    /// arrives through [`Self::record_commands`].
    pub async fn ask_commands(&self) {
        let line = protocol::initialize(&format!("{}{}", protocol::COMMANDS, self.id));
        // A question, not an instruction: a session that has gone simply goes
        // unasked, and the next ask tries again.
        if let Err(why) = self.write_line(&line, "asking the session").await {
            tracing::debug!("{}: {why:#}", self.id);
        }
    }

    pub(super) fn record_commands(&self, commands: protocol::Commands) {
        self.state.lock().commands = Some(std::sync::Arc::new(commands));
    }

    /// What this session can be told, once it has said.
    pub fn commands(&self) -> Option<std::sync::Arc<protocol::Commands>> {
        self.state.lock().commands.clone()
    }

    /// Ask what the account has spent — [`protocol::get_usage`]; the answer arrives
    /// through [`Self::record_usage`].
    pub async fn ask_usage(&self) {
        let line = protocol::get_usage(&format!("usage-{}", self.id));
        // A question, not an instruction: a session that has gone simply goes
        // unasked, and the next ask tries again.
        if let Err(why) = self.write_line(&line, "asking the session").await {
            tracing::debug!("{}: {why:#}", self.id);
        }
    }
}

/// What became of a message offered.
#[derive(Debug, PartialEq)]
pub enum Offered {
    Wrote,
    /// It was already handed over, or read before this process: nothing written.
    Already,
    Refused(Refusal),
}

/// What became of a message taken back.
#[derive(Debug)]
pub enum Withdrawal {
    /// Taken back: the message, to give its words back, or none for one that
    /// never arrived.
    Back(Option<Message>),
    Refused(Refusal),
}

fn read_at(at: i64) -> Refusal {
    Refusal {
        why: Why::Read,
        at: Some(at),
    }
}
