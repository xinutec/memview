//! A workflow run, read from its journal and its agents' files.

use std::path::{Path, PathBuf};

use console::protocol::Event;
use console::workflows::{Meta, dir, is_agent, is_run, read, run};

/// Journal lines in the shape the harness writes them. The first run's `started`
/// lines carry label and phase; older runs' carry neither.
const LABELLED: &str = r#"{"type":"launched"}
{"type":"started","key":"v2:f8c2","agentId":"af4359a23fcc0ae33","label":"comments:core","phase":"Rewrite"}
{"type":"started","key":"v2:4922","agentId":"a3ce095293b0f6627","label":"comments:runner","phase":"Rewrite"}
{"type":"started","key":"v2:3935","agentId":"a2768deecd765667d","label":"verify:core","phase":"Verify"}
{"type":"result","key":"v2:f8c2","agentId":"af4359a23fcc0ae33","result":{"changed":3}}
"#;

fn no_meta(_: &str) -> anyhow::Result<Meta> {
    Ok(Meta::default())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("console-workflows-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn agents_are_grouped_by_phase_in_the_order_the_phases_began() {
    let read = read(LABELLED, no_meta).expect("read");
    let phases: Vec<_> = read
        .phases
        .iter()
        .map(|phase| {
            (
                phase.title.as_deref(),
                phase
                    .agents
                    .iter()
                    .map(|agent| (agent.label.as_deref(), agent.done))
                    .collect::<Vec<_>>(),
            )
        })
        .collect();
    assert_eq!(
        phases,
        vec![
            (
                Some("Rewrite"),
                vec![
                    (Some("comments:core"), true),
                    (Some("comments:runner"), false)
                ]
            ),
            (Some("Verify"), vec![(Some("verify:core"), false)]),
        ]
    );
}

#[test]
fn an_unlabelled_start_is_named_by_the_agents_meta_file() {
    let journal = r#"{"type":"started","key":"v2:2b97","agentId":"a2f85c2a5e5b2c50d"}"#;
    let read = read(journal, |agent| {
        assert_eq!(agent, "a2f85c2a5e5b2c50d");
        Ok(Meta {
            description: Some("comments:0".to_string()),
            phase: Some("Rewrite".to_string()),
        })
    })
    .expect("read");
    let phase = &read.phases[0];
    assert_eq!(phase.title.as_deref(), Some("Rewrite"));
    assert_eq!(phase.agents[0].label.as_deref(), Some("comments:0"));
}

#[test]
fn an_agent_started_twice_is_listed_once() {
    let journal = format!(
        "{LABELLED}{}",
        r#"{"type":"started","key":"v2:f8c2","agentId":"af4359a23fcc0ae33","label":"comments:core","phase":"Rewrite"}"#
    );
    assert_eq!(
        read(&journal, no_meta).expect("read").phases[0]
            .agents
            .len(),
        2
    );
}

#[test]
fn only_ids_of_the_harness_shape_become_paths() {
    assert!(is_run("wf_0480c7f6-60f"));
    for bad in ["wf_", "wf_../x", "../wf_1", "wf_a/b", "0480c7f6"] {
        assert!(!is_run(bad), "{bad}");
    }
    assert!(is_agent("a11a084e9cc5513e4"));
    for bad in ["", "..", "a/b", "a.b"] {
        assert!(!is_agent(bad), "{bad}");
    }
}

#[test]
fn a_run_lives_beside_the_transcript_of_the_session_that_launched_it() {
    assert_eq!(
        dir(Path::new("/p/-Users-x/c8d5.jsonl"), "wf_0480c7f6-60f"),
        Path::new("/p/-Users-x/c8d5/subagents/workflows/wf_0480c7f6-60f")
    );
}

/// An assistant line calling one tool, as a transcript records it.
fn calling(name: &str, command: &str) -> String {
    serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-09-24T14:28:00.000Z",
        "message": {"content": [{"type": "tool_use", "id": format!("toolu_{name}"), "name": name, "input": {"command": command}}]}
    })
    .to_string()
}

#[test]
fn a_working_agent_shows_its_last_call_and_a_finished_one_does_not() {
    let root = scratch("run");
    std::fs::write(root.join("journal.jsonl"), LABELLED).expect("journal");
    for agent in ["af4359a23fcc0ae33", "a3ce095293b0f6627"] {
        let lines = [calling("Read", "first"), calling("Bash", "cargo test")];
        std::fs::write(
            root.join(format!("agent-{agent}.jsonl")),
            lines.join("\n") + "\n",
        )
        .expect("agent");
    }
    let run = run(&root).expect("readable").expect("a journal");
    let rewrite = &run.phases[0].agents;
    assert!(rewrite[0].done && rewrite[0].latest.is_none());
    let latest = rewrite[1].latest.as_ref().expect("working");
    assert!(
        matches!(&latest.event, Event::Tool { name, .. } if name == "Bash"),
        "{latest:?}"
    );
    assert!(
        run.phases[1].agents[0].latest.is_none(),
        "no transcript yet"
    );
}

#[test]
fn no_journal_is_no_run() {
    assert!(run(&scratch("empty")).expect("readable").is_none());
}

#[test]
fn reading_forward_takes_whole_lines_and_a_half_written_one_later() {
    let path = scratch("since").join("agent-a1.jsonl");
    let first = calling("Read", "one");
    let second = calling("Bash", "two");
    let (half, rest) = second.split_at(20);
    std::fs::write(&path, format!("{first}\n{half}")).expect("write");

    let start = console::past::written(&path);
    assert_eq!(
        start,
        first.len() as u64 + 1,
        "the cursor stops at the last whole line"
    );

    let (events, to) = console::past::since(&path, 0);
    assert_eq!(events.len(), 1);
    assert_eq!(to, start);

    std::fs::write(&path, format!("{first}\n{half}{rest}\n")).expect("finish the line");
    let (events, end) = console::past::since(&path, to);
    assert!(
        matches!(&events[..], [timed] if matches!(&timed.event, Event::Tool { name, .. } if name == "Bash")),
        "{events:?}"
    );
    assert_eq!(console::past::since(&path, end).0.len(), 0);
}

#[test]
fn a_meta_file_that_does_not_parse_is_an_error_and_not_a_blank_label() {
    let root = scratch("corrupt");
    std::fs::write(
        root.join("journal.jsonl"),
        r#"{"type":"started","key":"v2:1","agentId":"a1"}"#,
    )
    .expect("journal");
    std::fs::write(root.join("agent-a1.meta.json"), "{not json").expect("meta");
    assert!(run(&root).is_err());
}
