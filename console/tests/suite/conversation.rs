//! `conversation` reads both sides of a transcript. These pin the one thing the
//! types cannot say: whose words a row actually holds.

use console::conversation::{COMPACTED, Voice, conversation};

fn row(uuid: &str, at: &str, text: &str) -> String {
    format!(
        r#"{{"type":"user","uuid":"{uuid}","timestamp":"{at}","message":{{"role":"user","content":"{text}"}}}}"#
    )
}

fn his(bytes: &[u8]) -> Vec<String> {
    conversation(bytes)
        .into_iter()
        .filter(|line| line.voice == Voice::Pippijn)
        .map(|line| line.text)
        .collect()
}

/// ⚠ **A compaction summary wears the user's role.** It reached `last --user` as
/// Pippijn's words, which is the one question that flag exists to answer.
#[test]
fn a_compaction_summary_is_not_something_he_said() {
    let typed = row(
        "a",
        "2026-08-31T10:00:00Z",
        "go check the closed tasks then.",
    );
    let injected = row(
        "b",
        "2026-08-31T10:01:00Z",
        &format!("{COMPACTED} and here is the summary."),
    );
    let bytes = format!("{typed}\n{injected}\n").into_bytes();

    assert_eq!(his(&bytes), vec!["go check the closed tasks then."]);
}

/// The other half of the same claim: dropping the summary must not drop the turn
/// after it, which is what a naive "skip the next row" would do.
#[test]
fn the_turn_after_a_summary_survives() {
    let injected = row("a", "2026-08-31T10:00:00Z", &format!("{COMPACTED}."));
    let after = row("b", "2026-08-31T10:01:00Z", "Send it");
    let bytes = format!("{injected}\n{after}\n").into_bytes();

    assert_eq!(his(&bytes), vec!["Send it"]);
}

/// An assistant turn is the session's, never his. Without this the two voices
/// are only separated by a field nobody asserts on.
#[test]
fn an_assistant_turn_belongs_to_the_session() {
    let said = r#"{"type":"assistant","uuid":"c","timestamp":"2026-08-31T10:02:00Z","message":{"content":[{"type":"text","text":"Ready."}]}}"#;
    let lines = conversation(format!("{said}\n").as_bytes());

    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].voice, Voice::Session);
    assert_eq!(lines[0].text, "Ready.");
}
