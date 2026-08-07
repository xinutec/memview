//! The rule that tells a conversation from the directory beside it.
//!
//! Both crates call this, and both had their own copy before. The console's copy
//! was missing and it shipped the bug; these pin the rule where it now lives, so
//! the next reader of either crate finds the guard in the same place as the
//! reason.

use std::path::Path;

use reader::transcript::is_transcript;

#[test]
fn a_conversation_is_the_file_with_the_extension() {
    assert!(is_transcript(Path::new(
        "/home/example/.claude/projects/some-project/9c8f2a11-dead-beef-cafe-000000000000.jsonl"
    )));
}

#[test]
fn the_directory_beside_it_is_not_one() {
    // ⚠ **The failure this exists for.** Claude Code puts a directory named for
    // the session right beside the session's transcript, holding `subagents/`
    // and `tool-results/`. Its file stem is the id, exactly like the file's, so
    // a stem match finds whichever `read_dir` returns first — and reading a
    // directory as a conversation yields no events and no error, which is
    // indistinguishable from a session that has only just begun.
    assert!(!is_transcript(Path::new(
        "/home/example/.claude/projects/some-project/9c8f2a11-dead-beef-cafe-000000000000"
    )));
}

#[test]
fn nothing_else_in_the_directory_counts() {
    // Project directories accumulate other things, and a reader that took
    // everything would hand each of them to a JSON parser and file the silence
    // as an empty conversation.
    for other in ["notes.md", "couse.json", "agents.json", "shell-snapshot.sh"] {
        assert!(
            !is_transcript(Path::new(other)),
            "{other} is not a conversation"
        );
    }
}

#[test]
fn the_vocabulary_is_what_the_cli_writes() {
    // Spelled once, so a viewer scanning bytes and a console parsing JSON cannot
    // drift on the name of a field they both read. Checked against the literal
    // the CLI emits rather than against itself, which would assert nothing.
    let needle = reader::transcript::name_needle(&reader::transcript::AGENT_NAME);
    assert_eq!(
        String::from_utf8(needle).expect("utf-8"),
        r#"{"type":"agent-name","agentName":""#
    );

    let needle = reader::transcript::name_needle(&reader::transcript::CUSTOM_TITLE);
    assert_eq!(
        String::from_utf8(needle).expect("utf-8"),
        r#"{"type":"custom-title","customTitle":""#
    );
}

#[test]
fn the_two_orders_are_opposite_on_purpose() {
    // ⚠ **This asserts a DECISION, not an implementation.** The console and the
    // viewer disagreed about which name wins, each with a confident rationale,
    // and the answer turned out to be that both were right: the CLI carries both
    // orders, split by what the name is for. Its session labeller reads
    // `agentName` first; its resume picker reads `customTitle` first and never
    // consults `agentName` at all.
    //
    // So a future reader who finds these opposed should not "fix" one to match
    // the other — that is the bug this pair replaced. Which order a call site
    // wants follows from the question it is answering, and is named at each of
    // them.
    let conversation: Vec<&str> = reader::transcript::AS_CONVERSATION
        .iter()
        .map(|line| line.line_type)
        .collect();
    let actor: Vec<&str> = reader::transcript::AS_ACTOR
        .iter()
        .map(|line| line.line_type)
        .collect();

    assert_eq!(conversation, ["custom-title", "agent-name"]);
    assert_eq!(actor, ["agent-name", "custom-title"]);

    let mut reversed = actor.clone();
    reversed.reverse();
    assert_eq!(
        conversation, reversed,
        "the two orders answer opposite questions and must stay opposite",
    );
}

#[test]
fn the_cli_s_own_description_is_not_a_name() {
    // `ai-title` is the CLI's summary of a conversation — "Review DICOM scan
    // documentation" — written once near the head of the file and never changed.
    // Fine as a caption; wrong on a page about who did the work. It is in neither
    // order, and the actor chain falls through to the session id instead.
    for order in [
        reader::transcript::AS_CONVERSATION,
        reader::transcript::AS_ACTOR,
    ] {
        assert!(
            !order.iter().any(|line| line.line_type.contains("ai-title")),
            "a description must not become a name",
        );
    }
}
