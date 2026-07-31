//! Agent mining, against synthetic transcripts.

use std::path::PathBuf;

use memview::agents::{Agent, scan};

/// A tool-call line as the transcripts actually write it.
fn call(tool: &str, path: &str, stamp: &str) -> String {
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"{tool}\",\"input\":{{\"file_path\":\"{path}\"}}}}]}}}}"
    )
}

/// Write transcripts into a fresh tree and mine them.
///
/// `sessions` maps a transcript id to its lines; the registry is a separate
/// directory, exactly as on the real machine.
fn mine(transcripts: &[(&str, Vec<String>)], registry: &[(&str, &str)]) -> Vec<Agent> {
    let dir: PathBuf = std::env::temp_dir().join(format!(
        "agents-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    let projects = dir.join("projects/-code");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    for (id, lines) in transcripts {
        std::fs::write(projects.join(format!("{id}.jsonl")), lines.join("\n")).unwrap();
    }
    for (pid, json) in registry {
        std::fs::write(sessions.join(format!("{pid}.json")), json).unwrap();
    }
    let found = scan(
        &dir.join("projects"),
        &sessions,
        "/code",
        "2026-07-31T00:00:00Z",
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    found.agents
}

#[test]
fn reads_and_writes_are_counted_apart_and_per_project() {
    // The distinction the view exists to make: consulting a repository is not
    // the same as being responsible for it.
    let agents = mine(
        &[(
            "s1",
            vec![
                call("Read", "/code/health/src/a.rs", "2026-07-01T10:00:00Z"),
                call("Read", "/code/pippijn/k8s/b.yaml", "2026-07-01T10:01:00Z"),
                call("Write", "/code/health/src/c.rs", "2026-07-01T10:02:00Z"),
                call("Edit", "/code/health/src/d.rs", "2026-07-01T10:03:00Z"),
            ],
        )],
        &[(
            "100",
            r#"{"pid":100,"sessionId":"s1","name":"health-agent"}"#,
        )],
    );

    assert_eq!(agents.len(), 1);
    let a = &agents[0];
    assert_eq!(a.name, "health-agent");
    assert_eq!(a.reads.get("health"), Some(&1));
    assert_eq!(a.reads.get("pippijn"), Some(&1));
    assert_eq!(a.writes.get("health"), Some(&2));
    assert_eq!(a.writes.get("pippijn"), None);
    assert_eq!(a.first, "2026-07-01T10:00:00Z");
    assert_eq!(a.last, "2026-07-01T10:03:00Z");
}

#[test]
fn where_an_agent_works_is_decided_by_writes_not_reads() {
    // `health` reads the monorepo more than anything else while doing its work
    // in `health`. Ranking by reads would call it a monorepo session, which is
    // not what it is.
    let agents = mine(
        &[(
            "s1",
            vec![
                call("Read", "/code/pippijn/a", "2026-07-01T10:00:00Z"),
                call("Read", "/code/pippijn/b", "2026-07-01T10:00:01Z"),
                call("Read", "/code/pippijn/c", "2026-07-01T10:00:02Z"),
                call("Write", "/code/health/x", "2026-07-01T10:00:03Z"),
            ],
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"health"}"#)],
    );

    assert_eq!(agents[0].home(), Some("health"));
}

#[test]
fn an_agent_that_has_only_read_still_reports_where() {
    let agents = mine(
        &[(
            "s1",
            vec![call("Read", "/code/observe/a", "2026-07-01T10:00:00Z")],
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"looker"}"#)],
    );

    assert_eq!(agents[0].home(), Some("observe"));
}

#[test]
fn work_outside_the_code_root_is_not_a_project() {
    // The scratchpad under /private/tmp is where every session writes throwaway
    // scripts, and the memory corpus is what every session reads. Counting
    // either would say the same thing about every agent, which is nothing.
    let agents = mine(
        &[(
            "s1",
            vec![
                call("Write", "/private/tmp/scratch/x.py", "2026-07-01T10:00:00Z"),
                call(
                    "Read",
                    "/elsewhere/.claude/memory/m.md",
                    "2026-07-01T10:00:01Z",
                ),
                call("Write", "/code/real/x", "2026-07-01T10:00:02Z"),
            ],
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"tidy"}"#)],
    );

    assert_eq!(agents[0].writes.len(), 1);
    assert_eq!(agents[0].writes.get("real"), Some(&1));
    assert!(agents[0].reads.is_empty());
}

#[test]
fn the_registry_beats_the_name_the_transcript_remembers() {
    // The in-transcript reminder is written once and goes stale on a rename;
    // the registry is live. A renamed session must not show its old name.
    let lines = vec![
        r#"{"type":"user","message":{"content":"the user named this session \"old-name\""}}"#
            .to_string(),
        call("Write", "/code/thing/x", "2026-07-01T10:00:00Z"),
    ];
    let agents = mine(
        &[("s1", lines)],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"new-name"}"#)],
    );

    assert_eq!(agents[0].name, "new-name");
}

#[test]
fn a_session_the_registry_forgot_falls_back_to_the_transcript_then_the_id() {
    let named = vec![
        r#"{"type":"user","message":{"content":"the user named this session \"remembered\""}}"#
            .to_string(),
        call("Write", "/code/thing/x", "2026-07-01T10:00:00Z"),
    ];
    let anonymous = vec![call("Write", "/code/thing/y", "2026-07-01T10:00:00Z")];
    let agents = mine(&[("s1", named), ("s2", anonymous)], &[]);

    let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"remembered"), "{names:?}");
    // Unnamed sessions keep their id rather than pooling into one bucket:
    // several distinct agents under a single "unknown" label would be a claim
    // about the work that nothing supports.
    assert!(names.contains(&"s2"), "{names:?}");
}

#[test]
fn several_tool_calls_on_one_line_all_count() {
    // A batched turn opens several files in one message. Counting it once would
    // understate exactly the agents that work hardest.
    let line = format!(
        "{{\"message\":{{\"content\":[{}]}}}}",
        (0..3)
            .map(|i| format!(
                "{{\"type\":\"tool_use\",\"name\":\"Read\",\"input\":{{\"file_path\":\"/code/health/f{i}\"}}}}"
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let agents = mine(
        &[("s1", vec![line])],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"busy"}"#)],
    );

    assert_eq!(agents[0].reads.get("health"), Some(&3));
}

#[test]
fn one_name_over_several_transcripts_is_one_agent() {
    // A resumed session writes a second transcript; it is still one agent.
    let agents = mine(
        &[
            (
                "s1",
                vec![call("Write", "/code/thing/a", "2026-07-01T10:00:00Z")],
            ),
            (
                "s2",
                vec![call("Write", "/code/thing/b", "2026-07-02T10:00:00Z")],
            ),
        ],
        &[
            ("100", r#"{"pid":100,"sessionId":"s1","name":"same"}"#),
            ("101", r#"{"pid":101,"sessionId":"s2","name":"same"}"#),
        ],
    );

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].transcripts, 2);
    assert_eq!(agents[0].writes.get("thing"), Some(&2));
    assert_eq!(agents[0].first, "2026-07-01T10:00:00Z");
    assert_eq!(agents[0].last, "2026-07-02T10:00:00Z");
}
