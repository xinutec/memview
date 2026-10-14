//! Work a tool call left running: how a launch, an ending and a kill are read
//! off what the tools answer and what the harness notifies.

use serde::{Deserialize, Serialize};

use super::{Event, between};

/// What one event says about work left running — a decision, testable alone.
#[derive(Debug, PartialEq, Eq)]
pub enum Running {
    /// A call was left running. Both names are kept: a notification names the
    /// `tool`, a kill names the `task`.
    Began {
        tool: String,
        task: Option<String>,
        /// The longest the work said it would last, in milliseconds.
        expires_in: Option<i64>,
    },
    /// The harness says one has finished, by whichever name the notification gave.
    Ended(Named),
    /// A task was stopped from here — the one ending reported nowhere else.
    Killed(String),
    /// Nothing that was running still is.
    Gone,
    /// This event says nothing about background work.
    Quiet,
}

/// Which of a background task's two names an ending speaks: the call (the count's
/// key) or the task.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Named {
    /// The `tool_use` id of the call that started the work.
    Call(String),
    /// The task id the harness gave it, which kills and monitor timeouts speak.
    Task(String),
}

/// Read one event for what it says about work left running. `Joined` resets the
/// count; `Started` does not, since every turn opens with one.
pub fn running(event: &Event) -> Running {
    match event {
        Event::ToolResult { id, detail, .. } => match (detached(detail), stopped(detail)) {
            (Some(task), _) => Running::Began {
                tool: id.clone(),
                task,
                expires_in: declared_expiry(detail),
            },
            (None, Some(task)) => Running::Killed(task),
            (None, None) => Running::Quiet,
        },
        Event::Background { tool, task, .. } => match (tool, task) {
            (Some(call), _) => Running::Ended(Named::Call(call.clone())),
            (None, Some(task)) => Running::Ended(Named::Task(task.clone())),
            // `finished` builds no such event.
            (None, None) => Running::Quiet,
        },
        Event::Joined { .. } => Running::Gone,
        _ => Running::Quiet,
    }
}

/// The task id a tool's answer says it left running — `Some(None)` when the id
/// cannot be read. Read off the answer's opening words, not the arguments: a
/// timed-out foreground command is backgrounded too, and a quoting result is not.
fn detached(said: &str) -> Option<Option<String>> {
    /// How a result opens when it left work running, and what precedes its id.
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
        // Bash, backgrounded at its timeout; the varying timeout precedes the id.
        Opening {
            opens: "Command did not complete within its",
            before_id: "and was moved to the background (ID: ",
        },
        // Agent; its id is on the next line, matched here and rendered nowhere.
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

/// The deadline a monitor declares, in milliseconds: `expires in 30m`, or the older
/// `timeout 1800000ms`. Passing it ends the work whatever its notice says.
fn declared_expiry(said: &str) -> Option<i64> {
    let opened = said.strip_prefix("Monitor started (task ")?;
    if let Some((_, rest)) = opened.split_once("timeout ") {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        return rest[digits.len()..]
            .starts_with("ms")
            .then(|| digits.parse().ok())
            .flatten();
    }
    let rest = opened.split_once("expires in ")?.1;
    let span: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    let (digits, unit) = span.split_at(span.find(|c: char| !c.is_ascii_digit())?);
    let count: i64 = digits.parse().ok()?;
    let unit_ms = match unit {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        _ => return None,
    };
    Some(count * unit_ms)
}

/// The task a kill ended, from its JSON answer's opening.
fn stopped(said: &str) -> Option<String> {
    id_at(said.strip_prefix(r#"{"message":"Successfully stopped task: "#)?)
}

/// The alphanumeric id starting here.
fn id_at(rest: &str) -> Option<String> {
    let id: String = rest
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect();
    (!id.is_empty()).then_some(id)
}

/// The call a task notification is about, and how it ended.
pub(super) fn finished(text: &str) -> Option<Event> {
    if !is_notification(text) {
        return None;
    }
    let status = between(text, "<status>", "</status>").map(crate::named::named);
    // The usual case: named by the call.
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

/// Whether a notification naming no call is an ending: a monitor's timeout, or the
/// harness giving up on an agent. Matched whole, so a quoting monitor cannot end itself.
fn ends_without_naming_the_call(text: &str) -> bool {
    const TIMED_OUT: &str = "[Monitor timed out — re-arm if needed.]";
    const NO_RECORD: &str = "No completion record was found for background agent";
    between(text, "<event>", "</event>").is_some_and(|event| event == TIMED_OUT || expired(event))
        || between(text, "<summary>", "</summary>").is_some_and(|said| said.starts_with(NO_RECORD))
}

/// The newer words for a monitor's timeout, whole; the advice to widen the filter
/// follows only silence:
///
/// `[Monitor expired after 5m with no events delivered. Re-arm it if you still
/// need the watch — and widen the filter if silence was unexpected.]`
///
/// `[Monitor expired after 30m with 5 events delivered. Re-arm it if you still
/// need the watch.]`
fn expired(event: &str) -> bool {
    const BEFORE: &str = "[Monitor expired after ";
    const RE_ARM: &str = " delivered. Re-arm it if you still need the watch";
    const AFTER_SILENCE: &str = " — and widen the filter if silence was unexpected.]";
    const AFTER_EVENTS: &str = ".]";
    let Some(rest) = event.strip_prefix(BEFORE) else {
        return false;
    };
    let (rest, silent) = match rest.strip_suffix(AFTER_SILENCE) {
        Some(rest) => (rest, true),
        None => match rest.strip_suffix(AFTER_EVENTS) {
            Some(rest) => (rest, false),
            None => return false,
        },
    };
    let Some((duration, delivered)) = rest
        .strip_suffix(RE_ARM)
        .and_then(|rest| rest.split_once(" with "))
    else {
        return false;
    };
    let delivered_fits = match silent {
        true => delivered == "no events",
        false => delivered
            .strip_suffix(" events")
            .or_else(|| delivered.strip_suffix(" event"))
            .is_some_and(number),
    };
    delivered_fits && duration.strip_suffix(['s', 'm', 'h']).is_some_and(number)
}

/// Digits, at least one.
fn number(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// Whether this text is a task notification — even one that will not parse, which
/// must still not become a prompt. It must end the message, or a quoting summary
/// would match.
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub label: Option<String>,
    /// The harness's task id, which is what a kill names.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub task: Option<String>,
}

/// How long a label may be before it is cut.
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
