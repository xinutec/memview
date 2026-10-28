//! What the console writes to a session: messages, pictures, decisions and
//! control requests, and the replies it reads back.

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use super::state::{Held, Unread};

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

    /// Send a message. A slash command sent mid-turn is held, under the lock that
    /// clears `working`, so a turn ending meanwhile cannot strand it.
    pub async fn send(&self, text: &str) -> Result<()> {
        self.send_with(text, None).await
    }

    /// [`Self::send`], under the client's id for the message — [`crate::sent`].
    pub async fn send_with(&self, text: &str, id: Option<&str>) -> Result<()> {
        let parked = {
            let mut state = self.state.lock();
            let parking = state.working && protocol::is_command(text);
            if parking {
                state.held.push_back(Held {
                    text: text.to_string(),
                    id: id.map(str::to_string),
                });
            }
            parking
        };
        if parked {
            return Ok(());
        }
        self.write_line(&protocol::prompt(text, id), "writing to the session")
            .await?;
        // Announced on the way in: the CLI may not read it for minutes.
        if protocol::is_command(text) {
            self.push(Event::Command {
                text: text.to_string(),
            });
        } else {
            // In flight until replayed — all [`Self::deaf`] goes on.
            self.state.lock().unread.push_back(Unread {
                text: text.to_string(),
                at: now(),
                id: id.map(str::to_string),
            });
            self.push(Event::Accepted {
                text: text.to_string(),
                id: id.map(str::to_string),
            });
        }
        // Held even if the CLI never echoes it.
        let mut state = self.state.lock();
        if state.asked.is_none() && !state.origin_read {
            state.asked = Some(text.to_string());
        }
        Ok(())
    }

    /// Send the commands held for this turn's end, one at a time through
    /// [`Self::send_with`], so a cancel mid-drain is honoured. A failed write stops it.
    pub async fn release_held(&self) -> Result<()> {
        loop {
            let next = {
                let mut state = self.state.lock();
                state.held.pop_front()
            };
            let Some(command) = next else { return Ok(()) };
            self.send_with(&command.text, command.id.as_deref()).await?;
        }
    }

    /// Take back a held command by its exact text. False when already released.
    pub fn forget_held(&self, text: &str) -> bool {
        let mut state = self.state.lock();
        let Some(at) = state.held.iter().position(|held| held.text == text) else {
            return false;
        };
        state.held.remove(at);
        true
    }

    /// Take back a held command by the client's id. False when it is not held.
    pub fn take_back(&self, id: &str) -> bool {
        {
            let mut state = self.state.lock();
            let Some(at) = state
                .held
                .iter()
                .position(|held| held.id.as_deref() == Some(id))
            else {
                return false;
            };
            state.held.remove(at);
        }
        self.push(Event::Withdrawn { id: id.to_string() });
        true
    }

    /// Where a message this process was given stands, by the client's id. What it
    /// cannot see — a message an earlier process wrote — the transcript answers.
    pub fn whereabouts(&self, id: &str) -> crate::sent::Where {
        let state = self.state.lock();
        if let Some(at) = state.echoed.get(id) {
            crate::sent::Where::Delivered { at: *at }
        } else if state.held.iter().any(|held| held.id.as_deref() == Some(id)) {
            crate::sent::Where::Held
        } else if state.unread.iter().any(|it| it.id.as_deref() == Some(id)) {
            crate::sent::Where::Unread
        } else {
            crate::sent::Where::Nowhere
        }
    }

    /// Ask the CLI to take back a message it holds in its queue: true when it
    /// did, false once a turn has taken it in. Every client is told it is gone.
    pub async fn withdraw_queued(&self, id: &str) -> Result<bool> {
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
        let cancelled = match tokio::time::timeout(WITHDRAW_PATIENCE, answer).await {
            Ok(Ok(cancelled)) => cancelled,
            Ok(Err(_)) => anyhow::bail!("the session ended before it answered"),
            Err(_) => {
                self.withdrawals.lock().remove(&request);
                anyhow::bail!("the session did not answer in time")
            }
        };
        if cancelled {
            self.state
                .lock()
                .unread
                .retain(|it| it.id.as_deref() != Some(id));
            self.push(Event::Withdrawn { id: id.to_string() });
        }
        Ok(cancelled)
    }

    /// Hand a withdrawal's answer to whoever is waiting for it.
    pub(super) fn answer_withdrawal(&self, request: &str, cancelled: bool) {
        if let Some(waiting) = self.withdrawals.lock().remove(request) {
            let _ = waiting.send(cancelled);
        }
    }

    /// Whether `event` is the echo of a message already echoed: the CLI echoes a
    /// retried id but records and answers it once. The first echo is noted.
    pub fn echoed_again(&self, event: &Event) -> bool {
        let (Event::Prompt { id: Some(id), .. } | Event::Shown { id: Some(id), .. }) = event else {
            return false;
        };
        let mut state = self.state.lock();
        if state.echoed.contains_key(id) {
            return true;
        }
        state.echoed.insert(id.clone(), now());
        false
    }

    /// Show the session pictures with what was said about them —
    /// [`protocol::prompt_with_images`].
    pub async fn show(
        &self,
        text: &str,
        pictures: &[protocol::Picture<'_>],
        id: Option<&str>,
    ) -> Result<()> {
        let line = protocol::prompt_with_images(text, pictures, id);
        self.write_line(&line, "writing to the session").await?;
        let mut state = self.state.lock();
        if state.asked.is_none() && !state.origin_read {
            // Named by the words, not the base64.
            state.asked = Some(match (text.trim(), pictures.len()) {
                ("", 1) => "an image".to_string(),
                ("", many) => format!("{many} images"),
                (words, _) => words.to_string(),
            });
        }
        Ok(())
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
