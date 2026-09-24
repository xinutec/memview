//! A tool call's input, read once on the Mac into what a client draws and what
//! the console labels running work by. The client never reads a tool's raw
//! arguments: a tool whose input changes shape falls to [`Call::Other`] here,
//! in one place, rather than to a blank field on the phone.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a call does, as far as a person reading the transcript needs to know.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Call {
    /// A shell command.
    Bash {
        command: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        description: Option<String>,
    },
    /// One replacement in one file: `before` becomes `after`, at every occurrence
    /// when `everywhere`.
    Edit {
        path: String,
        before: String,
        after: String,
        everywhere: bool,
    },
    /// Questions the person answers by picking. Only when every question and every
    /// choice could be read: a half-read question would show fewer options than
    /// were offered, and a person cannot see what is missing.
    Question { questions: Vec<Question> },
    /// A workflow script, by the name it goes by.
    Workflow {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        name: Option<String>,
    },
    /// Any other call.
    Other {
        /// The one argument worth showing: the first of the fields that name what
        /// the call works on, else the argument names.
        shown: String,
        /// The caller's own sentence about the call, else what it runs or asks.
        #[serde(skip_serializing_if = "Option::is_none")]
        #[cfg_attr(feature = "ts", ts(optional))]
        described: Option<String>,
    },
}

/// One question, as the tool asked it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Question {
    pub question: String,
    /// A word or two naming the decision, for the chip above it. May be empty.
    pub header: String,
    /// Only an explicit `true`: taking anything else as several would let one tap
    /// answer a question that wanted more, and send before the person had finished.
    pub multi_select: bool,
    pub options: Vec<Choice>,
}

/// One thing that could be picked.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Choice {
    pub label: String,
    /// What picking it would mean. Often the only part worth reading.
    pub description: String,
}

/// The tools whose arguments are read for more than the one worth showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Tool {
    Bash,
    Edit,
    /// Asks the person rather than the machine; see [`crate::protocol::QUESTION_TOOL`].
    AskUserQuestion,
    Workflow,
    #[serde(other)]
    Other,
}

impl Tool {
    /// The tool a CLI name stands for, by serde's own reading of the name.
    pub fn named(name: &str) -> Tool {
        Tool::deserialize(serde::de::value::StrDeserializer::<serde::de::value::Error>::new(name))
            .unwrap_or(Tool::Other)
    }
}

/// The fields that name what a call works on, in the order one is chosen.
const SHOWN: [&str; 7] = [
    "file_path",
    "path",
    "command",
    "pattern",
    "url",
    "prompt",
    "description",
];

impl Call {
    pub fn read(tool: &str, input: &Value) -> Call {
        let text = |key: &str| {
            input
                .get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        let raw = |key: &str| input.get(key).and_then(Value::as_str).map(str::to_owned);
        match Tool::named(tool) {
            Tool::Bash => {
                if let Some(command) = text("command") {
                    return Call::Bash {
                        command,
                        description: text("description"),
                    };
                }
            }
            Tool::Edit => {
                if let (Some(path), Some(before), Some(after)) =
                    (text("file_path"), raw("old_string"), raw("new_string"))
                {
                    return Call::Edit {
                        path,
                        before,
                        after,
                        everywhere: input.get("replace_all") == Some(&Value::Bool(true)),
                    };
                }
            }
            Tool::AskUserQuestion => {
                if let Some(questions) = questions(input) {
                    return Call::Question { questions };
                }
            }
            Tool::Workflow => {
                return Call::Workflow {
                    name: workflow_name(input),
                };
            }
            Tool::Other => {}
        }
        let shown = SHOWN
            .iter()
            .find_map(|key| text(key))
            .or_else(|| {
                let names: Vec<&str> = input
                    .as_object()
                    .map(|fields| fields.keys().map(String::as_str).collect())
                    .unwrap_or_default();
                (!names.is_empty()).then(|| names.join(", "))
            })
            .unwrap_or_else(|| tool.to_owned());
        Call::Other {
            shown,
            described: text("description")
                .or_else(|| text("command"))
                .or_else(|| text("prompt")),
        }
    }

    /// A short sentence for what this call is doing: the caller's own description
    /// where it wrote one, else what it runs. `None` rather than a guess.
    pub fn label(&self) -> Option<&str> {
        match self {
            Call::Bash {
                command,
                description,
            } => Some(description.as_deref().unwrap_or(command)),
            Call::Workflow { name } => name.as_deref(),
            Call::Other { described, .. } => described.as_deref(),
            Call::Edit { .. } | Call::Question { .. } => None,
        }
    }
}

/// The name a workflow goes by: a saved one's `name`, else the `name` its script's
/// `meta` literal declares, else the script file's, which the harness writes as
/// `<name>-<run id>.js`.
fn workflow_name(input: &Value) -> Option<String> {
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    if let Some(name) = field("name") {
        return Some(name.to_owned());
    }
    if let Some(script) = field("script") {
        let meta = &script[script.find("meta")?..];
        let rest = meta[meta.find("name")? + "name".len()..].trim_start();
        let rest = rest.strip_prefix(':')?.trim_start();
        let quote = rest
            .chars()
            .next()
            .filter(|c| matches!(c, '\'' | '"' | '`'))?;
        let name = &rest[1..];
        return Some(name[..name.find(quote)?].to_owned());
    }
    let file = std::path::Path::new(field("scriptPath")?)
        .file_stem()?
        .to_str()?;
    Some(
        file.rsplit_once("-wf_")
            .map_or(file, |(name, _)| name)
            .to_owned(),
    )
}

/// Every question in the input, or `None` when any of them cannot be read.
fn questions(input: &Value) -> Option<Vec<Question>> {
    let raw = input.get("questions")?.as_array()?;
    if raw.is_empty() {
        return None;
    }
    raw.iter().map(question).collect()
}

fn question(value: &Value) -> Option<Question> {
    let asked = value.get("question")?.as_str().filter(|s| !s.is_empty())?;
    let options = value.get("options")?.as_array()?;
    if options.is_empty() {
        return None;
    }
    Some(Question {
        question: asked.to_owned(),
        header: value
            .get("header")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        multi_select: value.get("multiSelect") == Some(&Value::Bool(true)),
        options: options.iter().map(choice).collect::<Option<_>>()?,
    })
}

fn choice(value: &Value) -> Option<Choice> {
    Some(Choice {
        label: value
            .get("label")?
            .as_str()
            .filter(|s| !s.is_empty())?
            .to_owned(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    })
}
