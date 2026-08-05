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
    fn the_cli_talking_to_itself_is_not_your_message() {
        // A transcript files these as user turns, and replayed as prompts they
        // outnumber the real ones: `/exit`, `Goodbye!` and a system reminder all
        // reading as things the person said.
        for text in [
            "<command-name>/exit</command-name>",
            "<local-command-stdout>Goodbye!</local-command-stdout>",
            "<local-command-caveat>Caveat: …</local-command-caveat>",
            "<system-reminder>a nudge</system-reminder>",
        ] {
            let line = format!(
                r#"{{"type":"user","message":{{"role":"user","content":"{}"}}}}"#,
                text.replace('"', "'")
            );
            assert!(read_recorded(&line).is_empty(), "{text}");
        }
    }

    #[test]
    fn a_message_that_merely_mentions_a_tag_is_still_yours() {
        // Recognised by how they open, not by containing a tag anywhere — asking
        // about `<system-reminder>` is a thing a person does.
        let line = r#"{"type":"user","message":{"role":"user","content":"what is a <system-reminder> for?"}}"#;
        assert!(matches!(
            read_recorded(line).as_slice(),
            [Event::Prompt { .. }]
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

/// What a tool returned, and when a line says it happened.
///
/// Both were missing from the wire, and both are the sort of absence that reads
/// as a finished feature: a tool call showed a tick and no answer, and a
/// conversation from June looked like it had happened this morning.
mod detail {
    use console::protocol::{Event, read, read_recorded, recorded_at};

    /// A recorded tool result whose content is a bare string — 98.4% of them.
    fn result(content: &str, error: bool) -> String {
        format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","is_error":{error},"content":{}}}]}}}}"#,
            serde_json::to_string(content).expect("json")
        )
    }

    #[test]
    fn a_tool_result_carries_what_it_said() {
        let line = result("3 matches in src/main.rs", false);
        assert!(matches!(
            read_recorded(&line).as_slice(),
            [Event::ToolResult { id, ok, detail, cut }]
                if id == "t1" && *ok && detail == "3 matches in src/main.rs" && cut.is_none()
        ));
    }

    #[test]
    fn a_failure_carries_its_reason_and_not_just_its_verdict() {
        // The case this matters most for: `ok: false` and nothing else is a
        // session that stopped for a reason nobody can read.
        let line = result("Permission denied", true);
        assert!(matches!(
            read_recorded(&line).as_slice(),
            [Event::ToolResult { ok, detail, .. }] if !*ok && detail == "Permission denied"
        ));
    }

    #[test]
    fn the_live_reader_carries_it_too() {
        // Two readers, one vocabulary. A result that is legible replayed and
        // opaque live would be worse than either alone.
        let line = result("done", false);
        assert!(matches!(
            read(&line).as_slice(),
            [Event::ToolResult { detail, .. }] if detail == "done"
        ));
    }

    #[test]
    fn a_long_result_is_cut_and_admits_it() {
        let whole = "x".repeat(9000);
        let line = result(&whole, false);
        let seen = read_recorded(&line);
        let [Event::ToolResult { detail, cut, .. }] = seen.as_slice() else {
            panic!("expected one tool result");
        };
        assert_eq!(detail.chars().count(), 2000);
        assert_eq!(*cut, Some(9000), "a snippet has to say what it is part of");
    }

    #[test]
    fn a_cut_never_lands_inside_a_character() {
        // Every transcript here has some. Cut by bytes, the tail of the snippet
        // is not text at all, and it is the *client* that then fails to render.
        let whole = "é".repeat(3000);
        let line = result(&whole, false);
        let seen = read_recorded(&line);
        let [Event::ToolResult { detail, cut, .. }] = seen.as_slice() else {
            panic!("expected one tool result");
        };
        assert_eq!(detail.chars().count(), 2000);
        assert_eq!(*cut, Some(3000));
        assert!(detail.ends_with('é'));
    }

    #[test]
    fn a_result_that_is_a_list_of_blocks_reads_as_well_as_one_that_is_a_string() {
        // 1.6% of them, and they hold the images.
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"a page"},{"type":"image","source":{}}]}]}}"#;
        assert!(matches!(
            read_recorded(line).as_slice(),
            [Event::ToolResult { detail, .. }] if detail == "a page\n[an image]"
        ));
    }

    #[test]
    fn a_picture_says_so_rather_than_showing_nothing() {
        // Rendered as an empty result it is indistinguishable from a tool that
        // did nothing, and there is no way to tell those apart on screen.
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"image","source":{}}]}]}}"#;
        assert!(matches!(
            read_recorded(line).as_slice(),
            [Event::ToolResult { detail, .. }] if detail == "[an image]"
        ));
    }

    #[test]
    fn a_result_with_nothing_in_it_stays_empty() {
        // Not every tool answers in words. An empty string serialises away, so
        // the client is told nothing rather than told "".
        let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1"}]}}"#;
        assert!(matches!(
            read_recorded(line).as_slice(),
            [Event::ToolResult { detail, cut, .. }] if detail.is_empty() && cut.is_none()
        ));
    }

    #[test]
    fn a_line_says_when_it_happened() {
        let line = r#"{"type":"user","timestamp":"2026-08-03T10:04:00.000Z","message":{"role":"user","content":"hello"}}"#;
        assert_eq!(recorded_at(line), Some(1_785_751_440_000));
    }

    #[test]
    fn a_line_that_does_not_say_is_not_guessed_at() {
        // Stamping it with the clock would date a conversation from June today,
        // which is worse than showing no time at all.
        for line in [
            r#"{"type":"user","message":{"role":"user","content":"hello"}}"#,
            r#"{"type":"user","timestamp":"the other day","message":{"role":"user","content":"x"}}"#,
            "not json at all",
        ] {
            assert_eq!(recorded_at(line), None, "{line}");
        }
    }
}

#[test]
fn a_task_notification_is_not_something_the_person_said() {
    // The harness files these as user messages, in the same place a typed
    // instruction lands — so unfiltered they render as though Pippijn had said
    // them, which is how the console got a wall of XML in the transcript.
    let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"[SYSTEM NOTIFICATION]\n<task-notification>\n<task-id>b74zci1hw</task-id>\n<tool-use-id>toolu_011Cnk</tool-use-id>\n<status>completed</status>\n</task-notification>"}]}}"#;
    assert!(matches!(
        read(line).as_slice(),
        [Event::Background { tool, status }] if tool == "toolu_011Cnk" && status == "completed"
    ));
}

#[test]
fn a_notification_without_a_tool_call_names_nothing() {
    // Matched on the tool-use id rather than the task id, because that is what
    // ties it to the call that started it. One without is not usable and must
    // not become a prompt either.
    let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<task-notification>\n<task-id>b1</task-id>\n</task-notification>"}]}}"#;
    assert!(read(line).is_empty(), "neither an event nor a prompt");
}

#[test]
fn an_ordinary_message_is_still_a_prompt() {
    let line = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"do the thing"}]}}"#;
    assert!(matches!(read(line).as_slice(), [Event::Prompt { text }] if text == "do the thing"));
}

#[test]
fn context_is_read_per_message_rather_than_per_turn() {
    // ⚠ The result line's `usage` is the SUM over every request a turn made, so
    // a long turn reports more tokens than the window holds — 1.6M against 1M,
    // seen on the phone. The per-message usage is the context as it stood.
    let line = r#"{"type":"assistant","message":{"role":"assistant","usage":{"input_tokens":2,"cache_creation_input_tokens":1272,"cache_read_input_tokens":546967,"output_tokens":244},"content":[{"type":"text","text":"hello"}]}}"#;
    assert!(
        read(line)
            .iter()
            .any(|e| matches!(e, Event::Context { tokens } if *tokens == 548_241)),
        "the prompt is input + cache creation + cache read: {:?}",
        read(line)
    );
}

#[test]
fn a_message_that_says_nothing_about_tokens_reports_no_context() {
    let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}]}}"#;
    assert!(
        !read(line)
            .iter()
            .any(|e| matches!(e, Event::Context { .. }))
    );
}

#[test]
fn the_window_is_declared_on_the_result_line_and_nowhere_else() {
    // Only the result line says how big the window is; how full it is comes per
    // message. Both halves are needed and they arrive from different lines.
    let line = r#"{"type":"result","subtype":"success","total_cost_usd":1.0,"num_turns":1,"duration_ms":5,"modelUsage":{"claude-opus-5":{"contextWindow":1000000}}}"#;
    assert!(matches!(
        read(line).as_slice(),
        [Event::Turn {
            window: Some(1_000_000),
            ..
        }]
    ));
}

#[test]
fn a_replayed_transcript_carries_the_context_too() {
    // A session that has just been resumed or carried across an upgrade should
    // know how full it is straight away. The counts are in the transcript; the
    // recorded reader used to drop them, so the figure was blank until the next
    // turn ended.
    let line = r#"{"type":"assistant","message":{"role":"assistant","usage":{"input_tokens":2,"cache_creation_input_tokens":1272,"cache_read_input_tokens":546967,"output_tokens":244},"content":[{"type":"text","text":"hello"}]}}"#;
    let events = console::protocol::read_recorded(line);
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Context { tokens } if *tokens == 548_241)),
        "{events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(e, Event::Text { .. })),
        "and the words too"
    );
}

/// A `get_usage` control response, in the shape 2.1.221's own schema declares.
const USAGE_REPLY: &str = r#"{"type":"control_response","response":{"request_id":"usage-x","subtype":"success","response":{"rate_limits":{"five_hour":{"utilization":31.5,"resets_at":"2026-08-04T23:00:00Z"},"seven_day":{"utilization":66,"resets_at":"2026-08-07T02:00:00Z"},"seven_day_opus":null}}}}"#;

#[test]
fn the_usage_reply_gives_up_both_windows() {
    let mut found = console::protocol::usage_reply(USAGE_REPLY).expect("rate limits");
    found.sort_by(|a, b| a.0.cmp(&b.0));
    // A percentage on the wire, a fraction here — one place multiplies, so
    // nothing downstream has to know which it was given.
    assert_eq!(found[0], ("five_hour".to_string(), 0.315, Some(1785884400)));
    assert_eq!(found[1], ("seven_day".to_string(), 0.66, Some(1786068000)));
    // A window the plan does not have is not a window at zero.
    assert_eq!(found.len(), 2);
}

#[test]
fn an_ordinary_line_is_not_a_usage_reply() {
    // Every line of a conversation passes this on its way to being read, so it
    // has to say no to nearly all of them.
    assert!(console::protocol::usage_reply(r#"{"type":"assistant","message":{}}"#).is_none());
    assert!(console::protocol::usage_reply("not json at all").is_none());
}

#[test]
fn a_response_whose_shape_has_moved_yields_nothing() {
    // ⚠ The CLI calls `get_usage` experimental and says the shape may change.
    // When it does, the console must fall back to the dashboard rather than
    // report a window it has misread.
    let moved = r#"{"type":"control_response","response":{"response":{"rateLimits":{"five_hour":{"utilization":31.5}}}}}"#;
    assert!(console::protocol::usage_reply(moved).is_none());
    // And a window with no figure in it is not a window at zero.
    let empty = r#"{"type":"control_response","response":{"response":{"rate_limits":{"five_hour":{"resets_at":"2026-08-04T23:00:00Z"}}}}}"#;
    assert!(console::protocol::usage_reply(empty).is_none());
}

/// What a call answered, as the stream reports it.
fn answered(id: &str, said: &str) -> Event {
    Event::ToolResult {
        id: id.to_string(),
        ok: true,
        detail: said.to_string(),
        cut: None,
    }
}

#[test]
fn every_way_a_tool_says_it_left_something_running() {
    // ⚠ **Verbatim openings, taken off this machine's own transcripts.** The
    // decision about what counts is made from what a call *answers* rather than
    // from what it was asked to do, because `run_in_background` is a request only
    // `Bash` accepts — measured across 27,731 calls, it appears on nothing else.
    // These four are the phrases the CLI actually emits; see `protocol::detached`
    // for the precision and recall behind them.
    for (what, said) in [
        (
            "a shell command asked to detach",
            "Command running in background with ID: bh0ynhbpb. Output is being written to /tmp/x",
        ),
        (
            // ⚠ The one no rule about arguments could have found: its input says
            // `run_in_background: false`, because that is what was asked for.
            "a shell command moved there for outliving its timeout",
            "Command did not complete within its 120s timeout and was moved to the background",
        ),
        (
            "a subagent, which detaches unless told not to",
            "Async agent launched successfully. (This tool result is internal metadata",
        ),
        (
            "a monitor, the tool this whole rule came from",
            "Monitor started (task bajuqh4xe, timeout 1800000ms). You will be notified on each event",
        ),
    ] {
        assert_eq!(
            console::protocol::running(&answered("toolu_bg", said)),
            console::protocol::Running::Began("toolu_bg".to_string()),
            "{what}"
        );
    }
}

#[test]
fn an_ordinary_answer_says_nothing_about_it() {
    // The common case by a wide margin — 13,340 of the 13,858 calls measured —
    // and the one that matters most: a false match would be a count that never
    // comes down, where a missed one is only the undercount this replaced.
    for said in [
        "",
        "3",
        "done",
        "error: unknown flag",
        "wrote 40 lines to src/main.rs",
        // Near misses on purpose: talking about background work is not doing any.
        "the runner keeps a background sweep for gists",
        "Monitor the log and tell me when it settles",
    ] {
        assert_eq!(
            console::protocol::running(&answered("toolu_fg", said)),
            console::protocol::Running::Quiet,
            "{said:?}"
        );
    }
}

#[test]
fn the_harness_notification_is_what_closes_one() {
    // Named by the call that started it, which is why the two are matched by id
    // rather than by order — background work finishes out of order by nature.
    assert_eq!(
        console::protocol::running(&Event::Background {
            tool: "toolu_bg".to_string(),
            status: "completed".to_string(),
        }),
        console::protocol::Running::Ended("toolu_bg".to_string())
    );
}

#[test]
fn a_new_process_inherits_nothing() {
    // ⚠ Measured: a console restart left eleven phantom tasks in the count, all
    // of them started by a process that no longer existed. Their notifications
    // died with it, so nothing would ever have closed them.
    assert_eq!(
        console::protocol::running(&Event::Started {
            model: "claude-opus-5".to_string(),
            cwd: "/home/example/Code".to_string(),
            tools: 1,
        }),
        console::protocol::Running::Gone
    );
}

#[test]
fn the_seed_boundary_forgets_what_the_transcript_replayed() {
    // ⚠ Measured on `health`: five tasks reported running, every one from that
    // afternoon, the newest gone nine hours, the session with no children at
    // all. A replayed transcript is full of calls that were backgrounded once —
    // history, not now. `Joined` is pushed after the replay for exactly this.
    assert_eq!(
        console::protocol::running(&Event::Joined {
            earlier: 400,
            from: 12_345,
        }),
        console::protocol::Running::Gone
    );
}
