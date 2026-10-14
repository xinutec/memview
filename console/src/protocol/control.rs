//! What the console writes to the CLI — prompts, decisions and control requests —
//! and how it reads the replies to its own requests.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The tool that asks the person a question. Approving it unchanged is no answer —
/// see [`Answers`].
pub const QUESTION_TOOL: &str = "AskUserQuestion";

/// What was chosen: the question's text against the option label picked.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Answer {
    One(String),
    Many(Vec<String>),
}

/// Every answer to one [`QUESTION_TOOL`] call.
pub type Answers = std::collections::BTreeMap<String, Answer>;

/// A note on one question, beside the option picked — combined with it, unlike
/// [`Reply::response`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Annotation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub notes: Option<String>,
}

/// Notes against the questions of one call, by the question's own text.
pub type Annotations = std::collections::BTreeMap<String, Annotation>;

/// What a person said about a question: options picked, or words instead. The CLI
/// reports only `response` when both are sent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Answers::is_empty")]
    pub answers: Answers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub response: Option<String>,
    /// Notes beside the choices — see [`Annotation`].
    #[serde(default, skip_serializing_if = "Annotations::is_empty")]
    pub annotations: Annotations,
}

impl Reply {
    /// Whether there is anything to say; an empty reply is refused.
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
            && self.response.as_deref().unwrap_or("").trim().is_empty()
            && !self
                .annotations
                .values()
                .any(|note| !note.notes.as_deref().unwrap_or("").trim().is_empty())
    }
}

/// The answer to a `can_use_tool` question. An allow echoes the arguments back —
/// with the answers written in, for [`QUESTION_TOOL`], which reads them from there.
pub fn decision(
    id: &str,
    allowed: bool,
    input: &serde_json::Value,
    why: &str,
    reply: Option<&Reply>,
) -> String {
    let response = if allowed {
        serde_json::json!({"behavior": "allow", "updatedInput": answered(input, reply)})
    } else {
        serde_json::json!({"behavior": "deny", "message": why})
    };
    serde_json::json!({
        "type": "control_response",
        "response": {"subtype": "success", "request_id": id, "response": response},
    })
    .to_string()
}

/// The approved input with the answers written in; nothing added when there are none.
fn answered(input: &serde_json::Value, reply: Option<&Reply>) -> serde_json::Value {
    let Some(reply) = reply else {
        return input.clone();
    };
    let mut input = input.clone();
    if let Some(object) = input.as_object_mut() {
        if !reply.answers.is_empty() {
            object.insert("answers".to_string(), serde_json::json!(reply.answers));
        }
        // An empty key would override the choices with nothing.
        if let Some(said) = reply.response.as_deref().filter(|s| !s.trim().is_empty()) {
            object.insert("response".to_string(), serde_json::json!(said));
        }
        // A blank note would report an untouched question as answered.
        let notes: Annotations = reply
            .annotations
            .iter()
            .filter(|(_, note)| !note.notes.as_deref().unwrap_or("").trim().is_empty())
            .map(|(question, note)| (question.clone(), note.clone()))
            .collect();
        if !notes.is_empty() {
            object.insert("annotations".to_string(), serde_json::json!(notes));
        }
    }
    input
}

/// Rename a conversation over the control channel, which works mid-turn where
/// `/rename` would reach the model as words. The transcript gains a `custom-title`.
pub fn rename(request_id: &str, title: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "rename_session", "title": title},
    })
    .to_string()
}

pub fn set_mode(request_id: &str, mode: &crate::modes::Mode) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "set_permission_mode", "mode": mode.name()},
    })
    .to_string()
}

/// The request id of a mode change, one per session, so its answer is found.
pub const SET_MODE: &str = "set-mode-";

/// How a session answered a mode change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeReply {
    /// The mode the CLI says it is now in, not the one asked for.
    Now(crate::modes::Mode),
    /// The CLI refused, in its own words.
    Refused(String),
}

/// A session's answer to a mode change, if this line is one, matched on the
/// request id: the mode it is now in, or the refusal.
///
/// ```text
/// {"subtype":"success","request_id":"…","response":{"mode":"acceptEdits"}}
/// {"subtype":"error","request_id":"…","error":"Cannot set permission mode to …"}
/// ```
pub fn mode_reply(line: &str) -> Option<ModeReply> {
    let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
    if parsed.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let response = parsed.get("response")?;
    if !response.get("request_id")?.as_str()?.starts_with(SET_MODE) {
        return None;
    }
    match response.get("subtype")?.as_str()? {
        "success" => Some(ModeReply::Now(crate::modes::Mode::named(
            response.get("response")?.get("mode")?.as_str()?,
        ))),
        // A refusal with no words is still a refusal.
        "error" => Some(ModeReply::Refused(
            response
                .get("error")
                .and_then(|it| it.as_str())
                .unwrap_or("the session refused the change")
                .to_string(),
        )),
        _ => None,
    }
}

/// Ask the session what the account has spent — the only routine source, since
/// `rate_limit_event` speaks only at thresholds. Experimental in the CLI.
pub fn get_usage(request_id: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "get_usage"},
    })
    .to_string()
}

/// The rate limits out of any `control_response` carrying them. Read field by
/// field, so a moved shape yields nothing rather than something wrong.
pub fn usage_reply(line: &str) -> Option<UsageReply> {
    let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
    if parsed.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let limits = parsed
        .get("response")?
        .get("response")?
        .get("rate_limits")?
        .as_object()?;
    let fetched = limits
        .get("limits")
        .is_some_and(serde_json::Value::is_array);
    let mut found = Vec::new();
    for (window, seen) in limits {
        if window == MODEL_SCOPED {
            found.extend(scoped(seen));
            continue;
        }
        // A percentage here, a fraction on the stream: both become a fraction.
        let Some(pct) = seen.get("utilization").and_then(|it| it.as_f64()) else {
            continue;
        };
        found.push((window.clone(), pct / 100.0, resets_at(seen)));
    }
    (!found.is_empty()).then_some(UsageReply {
        windows: found,
        fetched,
    })
}

/// What a `get_usage` answer says about the rate limits.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageReply {
    /// Each window, by name, as a fraction and when it turns over (epoch seconds).
    pub windows: Vec<(String, f64, Option<i64>)>,
    /// Whether the CLI fetched this from the usage service — a measurement — rather
    /// than repeating old headers. Only the fetched shape has a `limits` list.
    pub fetched: bool,
}

/// Where the CLI files a window belonging to one model rather than to the plan.
const MODEL_SCOPED: &str = "model_scoped";

/// The per-model windows, named by the model; an unnamed entry is dropped.
fn scoped(value: &serde_json::Value) -> Vec<(String, f64, Option<i64>)> {
    let Some(entries) = value.as_array() else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| {
            let name = entry.get("display_name")?.as_str()?;
            let pct = entry.get("utilization")?.as_f64()?;
            Some((crate::usage::model_key(name), pct / 100.0, resets_at(entry)))
        })
        .collect()
}

/// When a window turns over, in epoch seconds, if it says.
fn resets_at(window: &serde_json::Value) -> Option<i64> {
    window
        .get("resets_at")
        .and_then(|it| it.as_str())
        .and_then(|it| OffsetDateTime::parse(it, &Rfc3339).ok())
        .map(|when| when.unix_timestamp())
}

/// One user message, as the CLI reads it on stdin; the caller adds the newline.
pub fn prompt(text: &str) -> String {
    serde_json::json!({
        "type": "user",
        "message": {"role": "user", "content": [{"type": "text", "text": text}]},
    })
    .to_string()
}

/// The phrase that ties a sent picture to the copy on disk. Written by
/// [`prompt_with_image`], read by [`shown`].
const ALSO_AT: &str = "the image is also at ";

/// The picture a user message carries, by file name, and the words without the
/// note that is addressed to the session.
pub fn shown(text: &str) -> (Option<String>, String) {
    let Some(open) = text.rfind(&format!("({ALSO_AT}")) else {
        return (None, text.to_string());
    };
    let from = open + ALSO_AT.len() + 1;
    let Some(shut) = text[from..].find(')') else {
        return (None, text.to_string());
    };
    let name = std::path::Path::new(&text[from..from + shut])
        .file_name()
        .and_then(|it| it.to_str())
        .map(String::from);
    let words = format!("{}{}", &text[..open], &text[from + shut + 1..]);
    (name, words.trim().to_string())
}

/// One user message carrying a picture, then what was said about it: a question
/// read first is answered from the question alone. The text names the console's
/// copy, which a session can open after compaction and [`shown`] finds on replay.
pub fn prompt_with_image(
    text: &str,
    media_type: &str,
    base64: &str,
    kept: &std::path::Path,
) -> String {
    let said = match text.trim() {
        "" => format!("({ALSO_AT}{})", kept.display()),
        words => format!("{words}\n\n({ALSO_AT}{})", kept.display()),
    };
    serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": [
                {"type": "image", "source": {"type": "base64", "media_type": media_type, "data": base64}},
                {"type": "text", "text": said},
            ],
        },
    })
    .to_string()
}
