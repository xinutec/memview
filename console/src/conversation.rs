//! Both sides of one conversation, read out of a transcript's bytes. A function
//! over bytes: nothing here opens a socket or spawns a process.

/// The first words of the compaction summary the CLI writes into a conversation;
/// a prefix, since the summary itself follows.
pub const COMPACTED: &str = "This session is being continued from a previous conversation";

/// Who said it.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Voice {
    User,
    Session,
}

/// One turn, from either side.
pub struct Line {
    pub at: String,
    pub voice: Voice,
    pub text: String,
}

/// The text of an assistant turn, or `None` for every other kind of row.
fn said(row: &serde_json::Value) -> Option<String> {
    if row["type"].as_str()? != "assistant" {
        return None;
    }
    let text = row["message"]["content"]
        .as_array()?
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let text = text.trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// Both sides of the conversation, in the order recorded; `reader` decides what
/// counts as a human turn.
pub fn conversation(bytes: &[u8]) -> Vec<Line> {
    let mut lines: Vec<Line> = reader::transcript::human_turns(bytes)
        .into_iter()
        // A compaction summary has the user's role but a model wrote it.
        .filter(|turn| !turn.text.starts_with(COMPACTED))
        .map(|turn| Line {
            at: turn.at,
            voice: Voice::User,
            text: turn.text,
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    for row in bytes.split(|byte| *byte == b'\n') {
        let Ok(row) = serde_json::from_slice::<serde_json::Value>(row) else {
            continue;
        };
        // The CLI rewrites earlier stretches into the file: keep the first copy.
        let uuid = row["uuid"].as_str().unwrap_or_default().to_string();
        if !uuid.is_empty() && !seen.insert(uuid) {
            continue;
        }
        if let Some(text) = said(&row) {
            lines.push(Line {
                at: row["timestamp"].as_str().unwrap_or_default().to_string(),
                voice: Voice::Session,
                text,
            });
        }
    }
    // ISO-8601 sorts as text; a queued turn sorts by when it was typed.
    lines.sort_by(|a, b| a.at.cmp(&b.at));
    lines
}
