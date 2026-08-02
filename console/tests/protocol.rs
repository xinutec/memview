//! What the console can read out of a real turn.
//!
//! The fixture is a genuine capture from CLI 2.1.220 (one prompt, one reply,
//! home paths scrubbed), not a hand-written approximation of one — the whole
//! risk in this module is believing the protocol has a shape it does not.

use console::protocol::{Event, read};

const TURN: &str = include_str!("fixtures/turn.jsonl");

fn events() -> Vec<Event> {
    TURN.lines().flat_map(read).collect()
}

#[test]
fn a_real_turn_reads_end_to_end() {
    let seen = events();

    // The session announced itself…
    assert!(
        matches!(seen.first(), Some(Event::Started { cwd, tools, .. }) if cwd == "/tmp/probe" && *tools > 0),
        "first event should be the session starting, got {:?}",
        seen.first()
    );
    // …the prompt came back…
    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::Prompt { text } if text.contains("hello"))),
        "the replayed prompt should be surfaced"
    );
    // …and the turn ended with what it cost.
    assert!(
        matches!(seen.last(), Some(Event::Turn { turns, cost_usd, .. }) if *turns == 1 && *cost_usd > 0.0),
        "last event should be the finished turn, got {:?}",
        seen.last()
    );
}

#[test]
fn the_reply_is_assembled_from_the_deltas_and_not_doubled() {
    // The same text arrives twice on the wire — streamed as deltas and repeated
    // in the completed message. Taking both would show the answer twice.
    let text: String = events()
        .iter()
        .filter_map(|e| match e {
            Event::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(text.trim(), "hello", "assembled reply, from deltas only");
}

#[test]
fn thinking_is_kept_apart_from_the_answer() {
    // Both arrive as content_block_delta and differ only by the delta's type; a
    // console that conflated them would print the model's reasoning as its reply.
    let thinking: String = events()
        .iter()
        .filter_map(|e| match e {
            Event::Thinking { text } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert!(
        thinking.contains("hello"),
        "the thinking was captured separately, got {thinking:?}"
    );
    assert!(
        !thinking.trim().is_empty() && thinking.trim() != "hello",
        "thinking is its own stream, not a copy of the answer"
    );
}

#[test]
fn the_rate_limit_window_is_a_status_and_not_a_percentage() {
    // Worth pinning: the design assumed the percentages had to come from the
    // statusLine hook, and this event is what the stream actually offers.
    let limits: Vec<_> = events()
        .into_iter()
        .filter_map(|e| match e {
            Event::Limit { window, status, .. } => Some((window, status)),
            _ => None,
        })
        .collect();
    assert_eq!(
        limits,
        vec![("five_hour".to_string(), "allowed".to_string())]
    );
}

#[test]
fn lines_it_does_not_know_are_survivable() {
    // The CLI grows message types between releases, and meeting one must not be
    // fatal — this is the property that keeps the console working across an
    // upgrade nobody told us about.
    assert!(read("{\"type\":\"something_new_in_2.2\",\"payload\":{}}").is_empty());
    assert!(read("not json at all").is_empty());
    assert!(read("").is_empty());
}

/// A recorded transcript is the same vocabulary read the other way round.
mod recorded {
    use console::protocol::{Event, read_recorded};

    #[test]
    fn assistant_text_survives_because_a_file_has_no_deltas() {
        // The trap: the live reader drops text from the completed message, since
        // the deltas already carried it. A transcript has only completed
        // messages, so the same rule there yields tool calls and silence.
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"the answer"}]}}"#;
        assert!(matches!(
            read_recorded(line).as_slice(),
            [Event::Text { text }] if text == "the answer"
        ));
    }

    #[test]
    fn thinking_survives_too() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"}]}}"#;
        assert!(matches!(
            read_recorded(line).as_slice(),
            [Event::Thinking { text }] if text == "hmm"
        ));
    }

    #[test]
    fn a_user_line_whose_content_is_a_bare_string_is_not_lost() {
        // Both shapes are in every transcript on this machine. Declared as a list
        // alone, this one becomes an empty list rather than an error — a turn
        // that silently vanishes.
        let line = r#"{"type":"user","message":{"role":"user","content":"do the thing"}}"#;
        assert!(matches!(
            read_recorded(line).as_slice(),
            [Event::Prompt { text }] if text == "do the thing"
        ));
    }

    #[test]
    fn a_tool_call_and_its_result_both_come_back() {
        let call = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}"#;
        let result = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","is_error":false}]}}"#;
        assert!(matches!(
            read_recorded(call).as_slice(),
            [Event::Tool { .. }]
        ));
        assert!(matches!(
            read_recorded(result).as_slice(),
            [Event::ToolResult { ok: true, .. }]
        ));
    }

    #[test]
    fn the_lines_that_belong_to_the_cli_are_left_alone() {
        // A transcript carries bridge lines, file snapshots and summaries. They
        // are the CLI's bookkeeping, not the conversation.
        for line in [
            r#"{"type":"bridge-session","sessionId":"x","bridgeSessionId":"cse_y"}"#,
            r#"{"type":"file-history-snapshot"}"#,
            r#"{"type":"summary","summary":"a compaction"}"#,
            "not json at all",
        ] {
            assert!(read_recorded(line).is_empty(), "{line}");
        }
    }
}
