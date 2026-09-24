//! What the console writes to the CLI — prompts, decisions and control requests —
//! and how it reads the replies to its own requests.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// The tool that asks the person a question rather than the machine a favour. It
/// arrives as an ordinary [`super::Event::Ask`], but approving it unchanged is not an
/// answer — see [`Answers`].
pub const QUESTION_TOOL: &str = "AskUserQuestion";

/// What was chosen: the question's own text, against the option label picked.
/// Labels, not indexes, since the CLI matches these against what it offered.
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

/// Something said about one question, beside the option picked. Unlike
/// [`Reply::response`] this combines rather than overrides: the CLI reports
/// `"<question>"=(no option selected) notes: …` for a note with no choice.
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

/// What a person said about a question: options picked, or words instead.
/// `response` and `answers` are alternatives — the CLI's result builder tests
/// `response` first and reports only that, so prose sent alongside
/// choices throws the choices away. The client is where that is made visible.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Reply {
    #[serde(default, skip_serializing_if = "Answers::is_empty")]
    pub answers: Answers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub response: Option<String>,
    /// Notes beside the choices — see [`Annotation`]. These travel with `answers`.
    #[serde(default, skip_serializing_if = "Annotations::is_empty")]
    pub annotations: Annotations,
}

impl Reply {
    /// Whether there is anything here to say. An empty reply would read to the
    /// session as "did not answer" — true, but better refused at the door.
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
            && self.response.as_deref().unwrap_or("").trim().is_empty()
            && !self
                .annotations
                .values()
                .any(|note| !note.notes.as_deref().unwrap_or("").trim().is_empty())
    }
}

/// The answer to a `can_use_tool` question, in the shape the CLI reads.
///
/// An allow must carry the arguments back: `updatedInput` lets a client edit what
/// it approves, and the console echoes the input unchanged. For [`QUESTION_TOOL`]
/// that edit is the whole point — its `call` reads `answers` out of its own
/// arguments, so approving unchanged yields *"The user did not answer the
/// questions."*
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

/// The approved input, with the answers written into it. Nothing invented when
/// there are none; a non-object input is left alone rather than replaced.
fn answered(input: &serde_json::Value, reply: Option<&Reply>) -> serde_json::Value {
    let Some(reply) = reply else {
        return input.clone();
    };
    let mut input = input.clone();
    if let Some(object) = input.as_object_mut() {
        if !reply.answers.is_empty() {
            object.insert("answers".to_string(), serde_json::json!(reply.answers));
        }
        // Only when there is something in it: writing the key at all says a choice was
        // overridden by nothing.
        if let Some(said) = reply.response.as_deref().filter(|s| !s.trim().is_empty()) {
            object.insert("response".to_string(), serde_json::json!(said));
        }
        // Blank notes are dropped: the CLI tests `notes` for truthiness, and an empty
        // one would make it report `(no option selected)` for a question nobody touched.
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

/// Rename a conversation, over the control channel — the only way to rename a
/// session that is working. `/rename` is input, and input arriving mid-turn is
/// parked and released as a prompt the model reads as words. A control request
/// is out-of-band: `success` comes back at once and the transcript gains
/// `{"type":"custom-title",…}`. `title`, and it must be a string — the CLI's own
/// validation; a `-p` session supports the subtype.
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

/// What the request id of a mode change looks like, so its answer is found. One
/// per session: the answer to the older of two changes is of no interest.
pub const SET_MODE: &str = "set-mode-";

/// How a session answered a mode change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeReply {
    /// The mode the CLI says it is now in, not the one asked for.
    Now(crate::modes::Mode),
    /// The CLI refused, in its own words, which are better than any of ours.
    Refused(String),
}

/// A session's answer to a mode change, if that is what this line is. Read it,
/// or the screen claims a mode the CLI never entered. The two shapes:
///
/// ```text
/// {"subtype":"success","request_id":"…","response":{"mode":"acceptEdits"}}
/// {"subtype":"error","request_id":"…","error":"Cannot set permission mode to
///   bypassPermissions because the session was not launched with
///   --dangerously-skip-permissions"}
/// ```
///
/// The success carries the mode, so this confirms rather than assumes it.
/// Matched on the request id, unlike [`usage_reply`].
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
        // A refusal with no words is still a refusal: the claim on screen comes down.
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

/// Ask the session what the account has spent. The only route to the routine
/// figures: `rate_limit_event` carries the percentage only when a threshold is
/// crossed, and the statusLine never runs for a headless session. `get_usage` is
/// the CLI's own answer — experimental, so nothing here insists on its shape.
pub fn get_usage(request_id: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "get_usage"},
    })
    .to_string()
}

/// The rate limits out of a `control_response`, if that is what this line is.
///
/// Read field by field: the CLI warns the shape may change, and a moved shape
/// yields no reading rather than a wrong one. A model's own allowance is not a
/// key beside the others — `model_scoped` is an array of `{display_name,
/// utilization, resets_at}`.
/// Not matched on a request id: any response carrying rate limits is an answer.
pub fn usage_reply(line: &str) -> Option<Vec<(String, f64, Option<i64>)>> {
    let parsed: serde_json::Value = serde_json::from_str(line).ok()?;
    if parsed.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let limits = parsed
        .get("response")?
        .get("response")?
        .get("rate_limits")?
        .as_object()?;
    let mut found = Vec::new();
    for (window, seen) in limits {
        if window == MODEL_SCOPED {
            found.extend(scoped(seen));
            continue;
        }
        // `utilization` is a percentage here, where the stream event's is a fraction.
        // Both become a fraction.
        let Some(pct) = seen.get("utilization").and_then(|it| it.as_f64()) else {
            continue;
        };
        found.push((window.clone(), pct / 100.0, resets_at(seen)));
    }
    (!found.is_empty()).then_some(found)
}

/// Where the CLI files a window belonging to one model rather than to the plan.
const MODEL_SCOPED: &str = "model_scoped";

/// The per-model windows out of that array, named by the model. An entry with no
/// name is dropped: a bar labelled after a guess is worse than none.
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

/// One user message, in the shape the CLI reads on stdin. `\n` terminated by
/// the caller — the CLI reads a line at a time.
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

/// The picture a user message carries, and the words with the note about it
/// taken out — the note is addressed to the session. Only the file name comes
/// back, never the directory.
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

/// One user message carrying a picture and what was said about it. The CLI
/// forwards an `image` block on stdin as the API defines one.
/// The picture first, the words after: a question read before the thing it is
/// about is answered from the question alone.
///
/// The text also names where the console kept its copy, so a session can open the
/// file after the image is compacted away — and it is how [`shown`] finds the
/// picture again on replay: a recorded image block holds base64 and no name.
/// Both share [`ALSO_AT`].
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
