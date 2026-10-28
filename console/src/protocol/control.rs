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

/// Take back a message the CLI holds in its queue, by its id: refused, as
/// `cancelled: false`, once a turn has taken it in.
pub fn withdraw(request_id: &str, message: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "cancel_async_message", "message_uuid": message},
    })
    .to_string()
}

/// The request id of a withdrawal, then the message's id, so its answer is found.
pub const WITHDRAW: &str = "withdraw-";

/// A session's answer to a withdrawal: the request it answers, and whether the
/// message was still waiting. An error is an answer too: nothing was taken back.
///
/// ```text
/// {"subtype":"success","request_id":"withdraw-…","response":{"cancelled":true}}
/// ```
pub fn withdraw_answer(parsed: &serde_json::Value) -> Option<(String, bool)> {
    if parsed.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let response = parsed.get("response")?;
    let request = response.get("request_id")?.as_str()?;
    if !request.starts_with(WITHDRAW) {
        return None;
    }
    let cancelled = response.get("subtype")?.as_str()? == "success"
        && response.get("response")?.get("cancelled")?.as_bool()?;
    Some((request.to_string(), cancelled))
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
    mode_answer(&serde_json::from_str(line).ok()?)
}

/// [`mode_reply`], of a line already parsed.
pub fn mode_answer(parsed: &serde_json::Value) -> Option<ModeReply> {
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

/// The request id prefix of [`initialize`], so its answer is told from others'.
pub const COMMANDS: &str = "commands-";

/// Ask the session what it can be told: its slash commands, and the values the
/// two that take a closed set accept. What the Agent SDK asks first; answering it
/// changes nothing in the session, and asking twice gets the same answer.
pub fn initialize(request_id: &str) -> String {
    serde_json::json!({
        "type": "control_request",
        "request_id": request_id,
        "request": {"subtype": "initialize"},
    })
    .to_string()
}

/// What a session can be told, for completing what is typed.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Commands {
    pub commands: Vec<SlashCommand>,
    /// What `/model` takes.
    pub models: Vec<ModelChoice>,
    /// What `/output-style` takes.
    pub output_styles: Vec<String>,
}

/// One slash command, as the CLI describes it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct SlashCommand {
    pub name: String,
    pub description: String,
    /// What it takes, in the CLI's notation: `<a|b>`, `[on|off]`, `[name]`. Empty
    /// for none.
    #[serde(skip_serializing_if = "String::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<String>", optional))]
    pub argument_hint: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(feature = "ts", ts(as = "Option<Vec<String>>", optional))]
    pub aliases: Vec<String>,
}

/// One value `/model` takes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ModelChoice {
    pub value: String,
    pub display_name: String,
    pub description: String,
    /// The `/effort` levels it supports; empty for a model with none.
    pub effort_levels: Vec<String>,
}

/// The answer to our [`initialize`], if this line is one. Read field by field, so
/// a moved shape yields less rather than something wrong.
pub fn commands_reply(line: &str) -> Option<Commands> {
    commands_answer(&serde_json::from_str(line).ok()?)
}

/// [`commands_reply`], of a line already parsed.
pub fn commands_answer(parsed: &serde_json::Value) -> Option<Commands> {
    if parsed.get("type")?.as_str()? != "control_response" {
        return None;
    }
    let response = parsed.get("response")?;
    if !response.get("request_id")?.as_str()?.starts_with(COMMANDS) {
        return None;
    }
    let body = response.get("response")?;
    let text = |v: &serde_json::Value, key: &str| {
        v.get(key)
            .and_then(|it| it.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let texts = |v: Option<&serde_json::Value>| -> Vec<String> {
        v.and_then(|it| it.as_array())
            .map(|all| {
                all.iter()
                    .filter_map(|it| it.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let commands = body
        .get("commands")?
        .as_array()?
        .iter()
        .filter(|c| c.get("name").and_then(|it| it.as_str()).is_some())
        .map(|c| SlashCommand {
            name: text(c, "name"),
            description: text(c, "description"),
            argument_hint: text(c, "argumentHint"),
            aliases: texts(c.get("aliases")),
        })
        .collect();
    let models = body
        .get("models")
        .and_then(|it| it.as_array())
        .into_iter()
        .flatten()
        .filter(|m| m.get("value").and_then(|it| it.as_str()).is_some())
        .map(|m| ModelChoice {
            value: text(m, "value"),
            display_name: text(m, "displayName"),
            description: text(m, "description"),
            effort_levels: texts(m.get("supportedEffortLevels")),
        })
        .collect();
    Some(Commands {
        commands,
        models,
        output_styles: texts(body.get("available_output_styles")),
    })
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
    usage_answer(&serde_json::from_str(line).ok()?)
}

/// [`usage_reply`], of a line already parsed.
pub fn usage_answer(parsed: &serde_json::Value) -> Option<UsageReply> {
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
/// With `id`, the CLI records it under that uuid and ignores a second message with
/// the same one — see [`crate::sent`].
pub fn prompt(text: &str, id: Option<&str>) -> String {
    identified(
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": [{"type": "text", "text": text}]},
        }),
        id,
    )
}

/// A user message with its `uuid`, when it has one.
fn identified(mut message: serde_json::Value, id: Option<&str>) -> String {
    if let (Some(id), Some(fields)) = (id, message.as_object_mut()) {
        fields.insert("uuid".to_string(), serde_json::Value::from(id));
    }
    message.to_string()
}

/// The phrase that ties a sent picture to the copy on disk. Written by
/// [`prompt_with_images`], read by [`shown`].
const ALSO_AT: &str = "the image is also at ";

/// The pictures a user message carries, by file name in the order sent, and the
/// words without the notes that are addressed to the session.
pub fn shown(text: &str) -> (Vec<String>, String) {
    let mut names = Vec::new();
    let mut words = text.to_string();
    while let Some(open) = words.rfind(&format!("({ALSO_AT}")) {
        let from = open + ALSO_AT.len() + 1;
        let Some(shut) = words[from..].find(')') else {
            break;
        };
        if let Some(name) = std::path::Path::new(&words[from..from + shut])
            .file_name()
            .and_then(|it| it.to_str())
        {
            names.push(name.to_string());
        }
        words = format!("{}{}", &words[..open], &words[from + shut + 1..]);
    }
    names.reverse();
    (names, words.trim().to_string())
}

/// One picture on its way to a session: what it is, its bytes as base64, and the
/// console's copy.
pub struct Picture<'a> {
    pub media_type: &'a str,
    pub base64: &'a str,
    pub kept: &'a std::path::Path,
}

/// One user message carrying pictures, then what was said about them: a question
/// read first is answered from the question alone. The text names each of the
/// console's copies, which a session can open after compaction and [`shown`] finds
/// on replay.
pub fn prompt_with_images(text: &str, pictures: &[Picture<'_>], id: Option<&str>) -> String {
    let notes = pictures
        .iter()
        .map(|it| format!("({ALSO_AT}{})", it.kept.display()))
        .collect::<Vec<_>>()
        .join("\n");
    let said = match text.trim() {
        "" => notes,
        words => format!("{words}\n\n{notes}"),
    };
    let content = pictures
        .iter()
        .map(|it| {
            serde_json::json!({
                "type": "image",
                "source": {"type": "base64", "media_type": it.media_type, "data": it.base64},
            })
        })
        .chain(std::iter::once(
            serde_json::json!({"type": "text", "text": said}),
        ))
        .collect::<Vec<_>>();
    identified(
        serde_json::json!({
            "type": "user",
            "message": {"role": "user", "content": content},
        }),
        id,
    )
}
