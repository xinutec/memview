//! Both sides of one conversation, read out of a transcript's bytes. A function
//! over bytes: nothing here opens a socket or spawns a process.

/// The first words of the compaction summary the CLI writes into a conversation;
/// a prefix, since the summary itself follows.
pub const COMPACTED: &str = "This session is being continued from a previous conversation";

/// Who said it.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Voice {
    Pippijn,
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

/// Both sides of the conversation, in the order recorded.
///
/// The human side is read by `reader`, which owns what counts as a human turn for
/// the whole workspace (a queued message lives in an `attachment` row, a
/// `tool_result` wears the user's role). The assistant side needs only the dedupe rule.
pub fn conversation(bytes: &[u8]) -> Vec<Line> {
    let mut lines: Vec<Line> = reader::transcript::human_turns(bytes)
        .into_iter()
        // A compaction summary wears his role and is not his: a model wrote it.
        // `human_turns` rightly keeps it; a reader asking for his words does not want it.
        .filter(|turn| !turn.text.starts_with(COMPACTED))
        .map(|turn| Line {
            at: turn.at,
            voice: Voice::Pippijn,
            text: turn.text,
        })
        .collect();

    let mut seen = std::collections::HashSet::new();
    for row in bytes.split(|byte| *byte == b'\n') {
        let Ok(row) = serde_json::from_slice::<serde_json::Value>(row) else {
            continue;
        };
        // The CLI rewrites earlier stretches back into the same file, so a linear read
        // returns some turns twice, the later copy degraded. Same rule as
        // `reader::transcript::human_turns`.
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
    // ISO-8601 in a fixed zone sorts as text. A queued turn is stamped when it was
    // ENQUEUED, so it can sort before the reply to the message ahead of it — that is
    // when it was typed, not a bug.
    lines.sort_by(|a, b| a.at.cmp(&b.at));
    lines
}
