//! Agent mining, against synthetic transcripts.

use std::path::PathBuf;

use memview::agents::{Agent, scan};

/// A tool whose calls the miner counts.
#[derive(Clone, Copy)]
enum Tool {
    Read,
    Write,
    Edit,
}

impl Tool {
    fn name(self) -> &'static str {
        match self {
            Tool::Read => "Read",
            Tool::Write => "Write",
            Tool::Edit => "Edit",
        }
    }

    /// The `input` object as this tool actually serialises it.
    ///
    /// **The shape is per-tool, and `file_path` is not always the first key.**
    /// `Edit` puts `replace_all` ahead of it and the payload after it. A fixture
    /// that wrote the path first for every tool is what hid a needle matching no
    /// `Edit` at all: 28,546 of them in the live corpus, every one counted as
    /// zero while the page called the total "writes".
    fn input(self, path: &str) -> String {
        match self {
            Tool::Read => format!("{{\"file_path\":\"{path}\"}}"),
            Tool::Write => format!("{{\"file_path\":\"{path}\",\"content\":\"x\"}}"),
            Tool::Edit => format!(
                "{{\"replace_all\":false,\"file_path\":\"{path}\",\"old_string\":\"a\",\"new_string\":\"b\"}}"
            ),
        }
    }
}

/// A tool-call line as the transcripts actually write it.
fn call(tool: Tool, path: &str, stamp: &str) -> String {
    let (name, input) = (tool.name(), tool.input(path));
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"{name}\",\"input\":{input}}}]}}}}"
    )
}

/// Write transcripts into a fresh tree and mine them.
///
/// `sessions` maps a transcript id to its lines; the registry is a separate
/// directory, exactly as on the real machine.
fn mine(transcripts: &[(&str, Vec<String>)], registry: &[(&str, &str)]) -> Vec<Agent> {
    mine_with(transcripts, &[], registry)
}

/// As [`mine`], plus transcripts of work a session dispatched.
///
/// `delegated` maps a relative path under the project root — the real nesting
/// is `<session>/subagents/agent-x.jsonl`, and again under
/// `subagents/workflows/<run>/` for workflow agents.
fn mine_with(
    transcripts: &[(&str, Vec<String>)],
    delegated: &[(&str, Vec<String>)],
    registry: &[(&str, &str)],
) -> Vec<Agent> {
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
    for (rel, lines) in delegated {
        let path = projects.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lines.join("\n")).unwrap();
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
                call(Tool::Read, "/code/health/src/a.rs", "2026-07-01T10:00:00Z"),
                call(
                    Tool::Read,
                    "/code/pippijn/k8s/b.yaml",
                    "2026-07-01T10:01:00Z",
                ),
                call(Tool::Write, "/code/health/src/c.rs", "2026-07-01T10:02:00Z"),
                call(Tool::Edit, "/code/health/src/d.rs", "2026-07-01T10:03:00Z"),
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
fn work_outside_the_code_root_is_not_a_project() {
    // The scratchpad under /private/tmp is where every session writes throwaway
    // scripts, and the memory corpus is what every session reads. Counting
    // either would say the same thing about every agent, which is nothing.
    let agents = mine(
        &[(
            "s1",
            vec![
                call(
                    Tool::Write,
                    "/private/tmp/scratch/x.py",
                    "2026-07-01T10:00:00Z",
                ),
                call(
                    Tool::Read,
                    "/elsewhere/.claude/memory/m.md",
                    "2026-07-01T10:00:01Z",
                ),
                call(Tool::Write, "/code/real/x", "2026-07-01T10:00:02Z"),
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
        call(Tool::Write, "/code/thing/x", "2026-07-01T10:00:00Z"),
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
        call(Tool::Write, "/code/thing/x", "2026-07-01T10:00:00Z"),
    ];
    let anonymous = vec![call(Tool::Write, "/code/thing/y", "2026-07-01T10:00:00Z")];
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
fn work_a_session_delegated_counts_as_its_own() {
    // A subagent has no name and no continuity; it exists because a named
    // session asked for it. Filing its edits separately would invent a one-shot
    // agent and subtract the work from the session responsible for it.
    let agents = mine_with(
        &[(
            "s1",
            vec![call(Tool::Write, "/code/thing/a", "2026-07-30T10:00:00Z")],
        )],
        &[
            (
                "s1/subagents/agent-aaa.jsonl",
                vec![call(Tool::Write, "/code/thing/b", "2026-07-30T10:05:00Z")],
            ),
            (
                // Workflow agents nest one level deeper again.
                "s1/subagents/workflows/wf_1/agent-bbb.jsonl",
                vec![call(Tool::Edit, "/code/other/c", "2026-07-30T10:06:00Z")],
            ),
        ],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"boss"}"#)],
    );

    assert_eq!(
        agents.len(),
        1,
        "delegated work must not become its own agent"
    );
    let a = &agents[0];
    assert_eq!(a.name, "boss");
    assert_eq!(a.transcripts, 1);
    assert_eq!(a.delegated, 2);
    assert_eq!(a.writes.get("thing"), Some(&2));
    assert_eq!(a.writes.get("other"), Some(&1));
}

#[test]
fn a_delegated_transcript_alone_still_lands_under_its_owner() {
    // Ordering matters: the subagent is read before nothing else establishes
    // the name. Without the registry it must still settle under the owning
    // session's id rather than the subagent's own filename.
    let agents = mine_with(
        &[],
        &[(
            "s9/subagents/agent-zzz.jsonl",
            vec![call(Tool::Write, "/code/thing/a", "2026-07-30T10:00:00Z")],
        )],
        &[],
    );

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "s9");
    assert_eq!(agents[0].transcripts, 0);
    assert_eq!(agents[0].delegated, 1);
}

#[test]
fn an_edit_counts_as_a_write_though_the_path_is_not_the_first_key() {
    // The defect this pins. `Edit` serialises `replace_all` before `file_path`,
    // so a needle expecting the path directly after the tool name matched none
    // of the 28,546 edits in the live corpus: reads looked entirely normal while
    // writes were missing the operation that does most of the changing, and the
    // page ranked agents on the remainder.
    let agents = mine(
        &[(
            "s1",
            vec![
                call(Tool::Edit, "/code/thing/a", "2026-07-30T10:00:00Z"),
                call(Tool::Write, "/code/thing/b", "2026-07-30T10:01:00Z"),
                call(Tool::Read, "/code/thing/c", "2026-07-30T10:02:00Z"),
            ],
        )],
        &[],
    );

    assert_eq!(
        agents[0].writes.get("thing"),
        Some(&2),
        "an Edit is a write"
    );
    assert_eq!(agents[0].reads.get("thing"), Some(&1));
}

#[test]
fn a_tool_call_carrying_no_path_does_not_borrow_the_next_one_s() {
    // Defensive: every Read/Write/Edit on the live corpus does carry a path, so
    // this is about what the lookup may not do rather than a shape seen in the
    // wild. Searching forward for the key without stopping at the next tool call
    // would credit a call that has no file of its own with the following call's.
    let line = concat!(
        r#"{"type":"assistant","timestamp":"2026-07-30T10:00:00Z","message":{"content":["#,
        r#"{"type":"tool_use","name":"Read","input":{"limit":10}},"#,
        r#"{"type":"tool_use","name":"Write","input":{"file_path":"/code/thing/a","content":"x"}}"#,
        r#"]}}"#
    );
    let agents = mine(&[("s1", vec![line.to_string()])], &[]);

    assert_eq!(agents[0].writes.get("thing"), Some(&1));
    assert_eq!(
        agents[0].reads.get("thing"),
        None,
        "the Read named no file of its own"
    );
}

#[test]
fn delegated_work_lands_under_a_name_the_transcript_supplied() {
    // The ordering invariant, and the reason a session's own transcript is read
    // before anything it dispatched: only that transcript carries the naming
    // reminder. Read a subagent first and the owner settles under its bare id,
    // splitting one agent into two rows — its own work under the name and its
    // delegated work under the uuid. Nothing else would fail.
    let own = vec![
        r#"{"type":"user","message":{"content":"the user named this session \"remembered\""}}"#
            .to_string(),
        call(Tool::Write, "/code/thing/a", "2026-07-30T10:00:00Z"),
    ];
    let agents = mine_with(
        &[("s1", own)],
        &[(
            "s1/subagents/agent-aaa.jsonl",
            vec![call(Tool::Write, "/code/thing/b", "2026-07-30T10:05:00Z")],
        )],
        &[],
    );

    let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(agents.len(), 1, "{names:?}");
    assert_eq!(agents[0].name, "remembered");
    assert_eq!(agents[0].delegated, 1);
    assert_eq!(agents[0].writes.get("thing"), Some(&2));
}

#[test]
fn a_subagent_quoting_a_name_does_not_name_itself() {
    // A subagent inherits its parent's context and can quote the reminder in it.
    // The reminder names the session that dispatched the work, so honouring it
    // here would file the work under a name of the parent's, arrived at by
    // accident — and would do it differently depending on what the parent had
    // been talking about.
    let agents = mine_with(
        &[],
        &[(
            "s9/subagents/agent-zzz.jsonl",
            vec![
                r#"{"type":"user","message":{"content":"the user named this session \"impostor\""}}"#
                    .to_string(),
                call(Tool::Write, "/code/thing/a", "2026-07-30T10:00:00Z"),
            ],
        )],
        &[],
    );

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "s9", "a subagent must not claim a name");
}

#[test]
fn one_busy_day_does_not_outrank_a_fortnight_of_steady_work() {
    // The case this ranking exists for. A session spent one afternoon making
    // many edits in a repository it never returned to, and has since worked
    // steadily somewhere else. Counting files says it belongs to the burst;
    // counting days present says it belongs where it keeps showing up.
    let mut lines = vec![];
    for i in 0..75 {
        lines.push(call(
            Tool::Write,
            &format!("/code/burst/f{i}"),
            "2026-07-21T10:00:00Z",
        ));
    }
    for day in ["25", "27", "29", "30", "31"] {
        lines.push(call(
            Tool::Write,
            "/code/steady/f",
            &format!("2026-07-{day}T10:00:00Z"),
        ));
    }

    let agents = mine(
        &[("s1", lines)],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"steady"}"#)],
    );
    let a = &agents[0];

    // The lifetime counts still tell the truth about what happened.
    assert_eq!(a.writes.get("burst"), Some(&75));
    assert_eq!(a.writes.get("steady"), Some(&5));

    // The ranking does not.
    let burst = a.recent_writes["burst"];
    let steady = a.recent_writes["steady"];
    assert!(
        steady > burst,
        "steady {steady} should outrank burst {burst}"
    );
}

#[test]
fn the_same_day_seen_twice_is_still_one_day() {
    // Presence is the unit. Were repeats counted, the burst would win again by
    // the back door.
    let agents = mine(
        &[(
            "s1",
            (0..20)
                .map(|i| {
                    call(
                        Tool::Write,
                        &format!("/code/p/f{i}"),
                        "2026-07-31T10:00:00Z",
                    )
                })
                .collect(),
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"a"}"#)],
    );

    assert_eq!(agents[0].writes.get("p"), Some(&20));
    // One day, and it is today, so it is worth exactly one.
    assert!((agents[0].recent_writes["p"] - 1.0).abs() < 1e-9);
}

#[test]
fn a_project_that_went_quiet_fades_but_does_not_vanish() {
    // Fading out of the ordering is right; disappearing is not. The lifetime
    // counts are the record, and a zero here would quietly contradict them.
    let agents = mine(
        &[(
            "s1",
            vec![call(Tool::Write, "/code/old/f", "2025-01-01T10:00:00Z")],
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"a"}"#)],
    );

    let score = agents[0].recent_writes["old"];
    assert!(score > 0.0 && score < 0.01, "{score}");
}

#[test]
fn one_name_over_several_transcripts_is_one_agent() {
    // A resumed session writes a second transcript; it is still one agent.
    let agents = mine(
        &[
            (
                "s1",
                vec![call(Tool::Write, "/code/thing/a", "2026-07-01T10:00:00Z")],
            ),
            (
                "s2",
                vec![call(Tool::Write, "/code/thing/b", "2026-07-02T10:00:00Z")],
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
