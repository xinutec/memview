//! What the console writes to a session: messages, pictures, decisions and
//! control requests, and the replies it reads back.

use anyhow::{Context, Result};
use tokio::io::AsyncWriteExt;

use super::state::Unread;
use super::{Heard, ResetsAt, Seen, Session, now};
use crate::protocol::{self, Event};

impl Session {
    /// Send a message to the session. A slash command sent mid-turn is held, not
    /// written — see [`super::State::held`] — under the same lock [`super::state::in_flight`] takes to
    /// clear `working`, so a turn ending beside this either drains what was parked
    /// or lets this write straight through.
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
        // Announced on the way in, not on the echo: the CLI may not read it for
        // minutes — see [`Event::Accepted`]. Prompt or command is decided here, since
        // only the text can say which will be echoed. See [`Event::Command`].
        if protocol::is_command(text) {
            self.push(Event::Command {
                text: text.to_string(),
            });
        } else {
            // In flight until the CLI replays it — the whole of what [`Self::deaf`] has to go on.
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

    /// Write the commands that were waiting for this turn to end, through
    /// [`Self::send`], so each takes the ordinary path and is recorded as it goes.
    /// One at a time, re-locked between each, so a cancel mid-drain is honoured. A
    /// failed write stops the drain: writing the next after a refusal would be pretending.
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

    /// Take back a command that is waiting, by its exact text. False is not an
    /// error: a second tap on a command already released has nothing to undo.
    pub fn forget_held(&self, text: &str) -> bool {
        let mut state = self.state.lock();
        let Some(at) = state.held.iter().position(|held| held == text) else {
            return false;
        };
        state.held.remove(at);
        true
    }

    /// Show the session a picture, with whatever was said about it. Not folded into
    /// [`Self::send`]: the two differ on the wire ([`protocol::prompt_with_image`]).
    pub async fn show(
        &self,
        text: &str,
        media_type: &str,
        base64: &str,
        kept: &std::path::Path,
    ) -> Result<()> {
        let line = protocol::prompt_with_image(text, media_type, base64, kept);
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
            // What it was opened for, when a picture is the first thing said — not the
            // base64.
            state.asked = Some(match text.trim() {
                "" => "an image".to_string(),
                words => words.to_string(),
            });
        }
        Ok(())
    }

    /// Answer a question the session is blocked on. Refusing carries a reason the
    /// session can act on. An unknown id is an error: the likeliest cause is two
    /// people looking at one session. `reply` is what was said about a
    /// [`protocol::QUESTION_TOOL`] call, refused for anything else — a console whose
    /// job is approving tool calls should not also rewrite what it approved.
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

    /// Rename the conversation — a control request, not `/rename`, which a working
    /// session hands to the model as words. See [`protocol::rename`]. Nothing is
    /// recorded on the way out: the CLI writes a `custom-title` line and the roster
    /// reads names from there ([`crate::past::about`]).
    ///
    /// This is the title, and it does not reach the session's peers. The CLI
    /// keeps the name other sessions address separately, and no control subtype
    /// writes it — only `-n` at spawn does, so a rename becomes visible to the rest
    /// of the fleet at the conversation's next resume and not before. See
    /// [`super::Spawn::name`].
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

    /// Change what this session may do without asking. Recorded optimistically —
    /// the CLI's answer is not waited for, see [`protocol::set_mode`] — and only
    /// after stdin has taken the line, so a failed write leaves the true mode on screen.
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
        // Kept so a refusal can put back the mode the session is really in. Only when
        // nothing is already outstanding, or the second change would record the first's
        // optimistic value as the truth.
        if state.restore.is_none() {
            state.restore = state.mode.clone();
        }
        // The old explanation goes with the old attempt.
        state.mode_refused = None;
        state.mode = Some(mode.clone());
        Ok(())
    }

    /// Take the CLI at its word about what mode it is in — the correction
    /// [`super::Session::set_mode`]'s optimism depends on, or a refused mode stays on
    /// screen. The confirmed mode comes
    /// from the reply, not from what was asked — see [`protocol::mode_reply`].
    pub(super) fn settle_mode(&self, reply: protocol::ModeReply) {
        let mut state = self.state.lock();
        match reply {
            protocol::ModeReply::Now(mode) => {
                state.mode = Some(mode);
                state.mode_refused = None;
            }
            protocol::ModeReply::Refused(why) => {
                tracing::info!("{}: the mode change was refused — {why}", self.id);
                // Back to what it was. `restore` is empty only for a reply to a change this
                // console did not make.
                if let Some(was) = state.restore.clone() {
                    state.mode = Some(was);
                }
                state.mode_refused = Some(why);
            }
        }
        state.restore = None;
    }

    /// Keep what the CLI answered about each window. Per window, not wholesale: an
    /// answer naming one window says nothing about another.
    pub(super) fn record_usage(&self, windows: Vec<(String, f64, Option<i64>)>) {
        let mut state = self.state.lock();
        let at = Heard(now());
        // Through [`crate::usage::remember`], not a blind insert: an answer from cached
        // headers is an echo, and must not overwrite a fresh `rate_limit_event`.
        crate::usage::remember(
            &mut state.spent,
            windows.into_iter().map(|(window, utilization, resets_at)| {
                (
                    window,
                    Seen {
                        utilization,
                        resets_at: resets_at.map(ResetsAt),
                        at,
                        measured: false,
                    },
                )
            }),
        );
    }

    /// Ask this session what the account has spent — [`protocol::get_usage`]. The
    /// answer arrives on stdout and is recorded as it passes [`Self::record_usage`].
    /// Failure is not propagated: a session that will not take the question is one
    /// whose figures the console does not have.
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
