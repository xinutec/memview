//! What the console writes to a session: messages, pictures, decisions and
//! control requests, and the replies it reads back.

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use super::state::Unread;
use super::{ResetsAt, Seen, Session, now};
use crate::protocol::{self, Event};

impl Session {
    /// Send a message. A slash command sent mid-turn is held, under the lock that
    /// clears `working`, so a turn ending meanwhile cannot strand it.
    pub async fn send(&self, text: &str) -> Result<()> {
        let parked = {
            let mut state = self.state.lock();
            let parking = state.working && protocol::is_command(text);
            if parking {
                state.held.push_back(text.to_string());
            }
            parking
        };
        if parked {
            return Ok(());
        }
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{}\n", protocol::prompt(text)).as_bytes())
            .await
            .context("writing to the session")?;
        stdin.flush().await.context("flushing to the session")?;
        drop(held);
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
            });
            self.push(Event::Accepted {
                text: text.to_string(),
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
    /// [`Self::send`], so a cancel mid-drain is honoured. A failed write stops it.
    pub async fn release_held(&self) -> Result<()> {
        loop {
            let next = {
                let mut state = self.state.lock();
                state.held.pop_front()
            };
            let Some(command) = next else { return Ok(()) };
            self.send(&command).await?;
        }
    }

    /// Take back a held command by its exact text. False when already released.
    pub fn forget_held(&self, text: &str) -> bool {
        let mut state = self.state.lock();
        let Some(at) = state.held.iter().position(|held| held == text) else {
            return false;
        };
        state.held.remove(at);
        true
    }

    /// Show the session pictures with what was said about them —
    /// [`protocol::prompt_with_images`].
    pub async fn show(&self, text: &str, pictures: &[protocol::Picture<'_>]) -> Result<()> {
        let line = protocol::prompt_with_images(text, pictures);
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("writing to the session")?;
        stdin.flush().await.context("flushing to the session")?;
        drop(held);
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
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("answering the session")?;
        stdin.flush().await.context("flushing the answer")?;
        drop(held);
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
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("renaming the session")?;
        stdin.flush().await.context("flushing the rename")?;
        Ok(())
    }

    /// Change what this session may do without asking. Recorded once written,
    /// before the CLI answers.
    pub async fn set_mode(&self, mode: &crate::modes::Mode) -> Result<()> {
        let line = protocol::set_mode(&format!("set-mode-{}", self.id), mode);
        let mut held = self.stdin.lock().await;
        let stdin = held
            .as_mut()
            .context("session is no longer accepting input")?;
        stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .context("asking the session to change mode")?;
        stdin.flush().await.context("flushing the mode change")?;
        drop(held);
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

    /// Ask what the account has spent — [`protocol::get_usage`]; the answer arrives
    /// through [`Self::record_usage`].
    pub async fn ask_usage(&self) {
        let line = protocol::get_usage(&format!("usage-{}", self.id));
        let mut held = self.stdin.lock().await;
        let Some(stdin) = held.as_mut() else { return };
        if stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .is_ok()
        {
            let _ = stdin.flush().await;
        }
    }
}
