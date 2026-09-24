//! Work a tool call left running: how a launch, an ending and a kill are read
//! off what the tools answer and what the harness notifies.

use serde::{Deserialize, Serialize};

use super::{Event, between};

/// What one event says about work that was left running. A decision rather than
/// a mutation, so the four cases can be tested without a session.
#[derive(Debug, PartialEq, Eq)]
pub enum Running {
    /// A call was started and left going, named by its own id. Both names kept
    /// because the two endings speak different ones: a notification names the `tool`,
    /// a kill names the `task`. `task` is `None` for a detach whose id could not be
    /// read.
    Began { tool: String, task: Option<String> },
    /// The harness says one has finished, by whichever name the notification gave.
    Ended(Named),
    /// A task was stopped from here, named by its task id — the one way work ends
    /// that reports nothing afterwards.
    Killed(String),
    /// Nothing that was running still is — as far as anybody can tell.
    Gone,
    /// This event says nothing about background work.
    Quiet,
}

/// Which of a background task's two names an ending speaks. A type rather than a
/// convention: the count is keyed by the call, so an ending naming it is a
/// removal, while one naming the task is a search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// The `tool_use` id of the call that started the work. What all but one
    /// kind of notification gives.
    Call(String),
    /// The task id the harness gave the work. What a kill speaks, and what a
    /// monitor's timeout speaks.
    Task(String),
}

/// Read one event for what it says about work left running.
///
/// `Joined` means "forget what you were counting": the replay is full of calls
/// backgrounded hours ago, and a re-seed replays that history and then joins.
/// `Started` is not that — the CLI emits an init line at the head of every turn.
///
/// A killed task is never reported finished, so the kill has to be read here.
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
        Event::Background { tool, task, .. } => match (tool, task) {
            (Some(call), _) => Running::Ended(Named::Call(call.clone())),
            (None, Some(task)) => Running::Ended(Named::Task(task.clone())),
            // Unreachable through `finished`, which builds no such event. Quiet rather than
            // a panic: an event naming nothing cannot bring a count down either way.
            (None, None) => Running::Quiet,
        },
        Event::Joined { .. } => Running::Gone,
        _ => Running::Quiet,
    }
}

/// What a tool says when it has left work running.
///
/// The call's answer, not its arguments: only `Bash` accepts `run_in_background`,
/// and a foreground command outliving its timeout is moved to the background with
/// its input still saying `false`. Every tool that detaches says so in the first
/// words it returns. A reworded CLI undercounts, which is the safe direction.
///
/// The phrase must open the result: a `contains` would count every result that
/// quotes one of these sentences — a grep, a `Read` of this module.
///
/// Returns the task id the harness gave the work; `Some(None)` is a detach whose
/// id could not be read.
fn detached(said: &str) -> Option<Option<String>> {
    /// One way a result opens when it has left work running: the words it starts
    /// with, and the marker its task id follows.
    struct Opening {
        opens: &'static str,
        before_id: &'static str,
    }
    const SAYS: [Opening; 5] = [
        // Bash, asked to detach.
        Opening {
            opens: "Command running in background with ID: ",
            before_id: "Command running in background with ID: ",
        },
        // Bash, moved to the background when it outran its timeout. The opening stops
        // short of the id because the timeout — a number that varies — sits between.
        Opening {
            opens: "Command did not complete within its",
            before_id: "and was moved to the background (ID: ",
        },
        // Agent, which runs in the background unless told otherwise. Its id is on the
        // following line, and the result asks that it never be repeated — so it is
        // matched on here and rendered nowhere.
        Opening {
            opens: "Async agent launched successfully",
            before_id: "\nagentId: ",
        },
        // Workflow, which has no foreground form.
        Opening {
            opens: "Workflow launched in background. Task ID: ",
            before_id: "Workflow launched in background. Task ID: ",
        },
        // Monitor.
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

/// The task a kill has just ended, when this result is one. The stopping call
/// answers in JSON, so this is a prefix of the document: `{"message":"Successfully
/// stopped task: …`.
fn stopped(said: &str) -> Option<String> {
    id_at(said.strip_prefix(r#"{"message":"Successfully stopped task: "#)?)
}

/// The id starting here, which runs until whatever ends it — a full stop, a
/// comma, a bracket or a space.
fn id_at(rest: &str) -> Option<String> {
    let id: String = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    (!id.is_empty()).then_some(id)
}

/// The tool call a task notification is about, and how it ended. Read for the
/// tool-use id, since that is what ties it to the call the client counted. Two
/// tags out of a known block; anything without both is not one of these.
pub(super) fn finished(text: &str) -> Option<Event> {
    if !is_notification(text) {
        return None;
    }
    let status = between(text, "<status>", "</status>").map(crate::named::named);
    // The ordinary case by a wide margin: the count is keyed by the call.
    if let Some(call) = between(text, "<tool-use-id>", "</tool-use-id>") {
        return Some(Event::Background {
            tool: Some(call.to_string()),
            task: None,
            status,
        });
    }
    let task = between(text, "<task-id>", "</task-id>")?;
    ends_without_naming_the_call(text).then(|| Event::Background {
        tool: None,
        task: Some(task.to_string()),
        status,
    })
}

/// How a background task ended, as its notification says.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Ended {
    Completed,
    Failed,
    Killed,
    Stopped,
    Unknown(String),
}

impl crate::named::Named for Ended {
    fn unknown(name: String) -> Self {
        Ended::Unknown(name)
    }
}

/// Whether a notification that names no call is nonetheless an ending. Most such
/// are a monitor's ordinary output; the endings are a monitor's timeout and the
/// harness giving up on an agent. A session's inherited tasks are a greeting.
/// Anchored to its own tag, never `contains`: a monitor quoting the timeout line
/// would end itself.
fn ends_without_naming_the_call(text: &str) -> bool {
    const TIMED_OUT: &str = "[Monitor timed out — re-arm if needed.]";
    const NO_RECORD: &str = "No completion record was found for background agent";
    between(text, "<event>", "</event>") == Some(TIMED_OUT)
        || between(text, "<summary>", "</summary>").is_some_and(|said| said.starts_with(NO_RECORD))
}

/// Whether this text is the harness reporting on a background task at all —
/// separate from parsing it, since one that cannot be parsed still must not
/// become a prompt. The block must end the message, or a compact summary quoting
/// one matches. Both ends, because the harness sometimes prefixes a banner.
pub(super) fn is_notification(text: &str) -> bool {
    text.contains("<task-notification>") && text.trim_end().ends_with("</task-notification>")
}

/// A background call, named the way somebody looking at the strip would name it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Called {
    /// The tool, as the CLI names it: `Bash`, `Monitor`, `Agent`.
    pub tool: String,
    /// A short label for what this call is doing, when the input carries one.
    /// `None` when it does not — better unlabelled than a guess.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub label: Option<String>,
    /// The harness's task id, which is what a kill names.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub task: Option<String>,
}

/// How long a label may be before it is cut: a `Bash` label falls back to the
/// command, which can run to several hundred characters.
const LABEL_MAX: usize = 60;

/// A call as the running strip names it: the tool, and [`crate::call::Call::label`]
/// flattened to one line and cut to [`LABEL_MAX`].
pub fn called(tool: &str, does: &crate::call::Call) -> Called {
    let label = does.label().map(|text| {
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
