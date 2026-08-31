//! Both sides of one conversation, read out of a transcript's bytes.
//!
//! Split out of the `sessions` binary so it can be tested through the public
//! API rather than from an inline test module, which is what `dev-lint`'s
//! `rust-test-module-in-src` asks for. Nothing here opens a socket or spawns a
//! process; it is a function over bytes.

/// The first words of the summary the CLI writes into a conversation when it is
/// compacted. Matched as a prefix because what follows it is the summary itself.
pub const COMPACTED: &str = "This session is being continued from a previous conversation";

/// Who said it. The session is named rather than called "assistant" because the
/// name is what a person asking about it uses.
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

/// Both sides of the conversation, in the order they were recorded.
///
/// ⚠ **The two sides are read by different rules and neither generalises.** What
/// counts as a human turn is five facts that have each cost somebody an
/// afternoon — a queued message lives in an `attachment` row and never in a
/// `user` one, a `tool_result` wears the user's role — and they live in
/// `reader`, which owns them for the whole workspace. The assistant side needs
/// only the dedupe rule, because nothing else wears its type.
pub fn conversation(bytes: &[u8]) -> Vec<Line> {
    let mut lines: Vec<Line> = reader::transcript::human_turns(bytes)
        .into_iter()
        // ⚠ **A compaction summary wears his role and is not his.** The CLI
        // injects it as a user turn when a conversation is compacted, so asking
        // this tool what Pippijn said returned a summary written by a model,
        // labelled with his name. `human_turns` is right not to filter it — it
        // answers what the human side of the file holds — but a reader asking
        // for his words is asking something narrower.
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
        // ⚠ The CLI rewrites earlier stretches back into the same file, so a
        // linear read returns some turns twice — and the later copy is the
        // degraded one. Same rule as `reader::transcript::human_turns`.
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
    // ISO-8601 in a fixed zone sorts as text, which is why the transcripts use
    // it. ⚠ A queued turn is stamped when it was ENQUEUED, so it can sort before
    // the reply to the message ahead of it — that is the truth about when it was
    // typed, not a bug to correct here.
    lines.sort_by(|a, b| a.at.cmp(&b.at));
    lines
}
