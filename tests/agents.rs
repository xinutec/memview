//! Agent mining, against synthetic transcripts.

use std::path::PathBuf;

use memview::agents::{Agent, Agents, scan};

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

/// A tool call whose result came back an error.
///
/// The tool ran and did nothing: an `Edit` fails when its `old_string` is not
/// there, and the file is left exactly as it was.
fn failed_call(tool: Tool, path: &str, stamp: &str) -> String {
    let (name, input) = (tool.name(), tool.input(path));
    format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{stamp}\",\"name\":\"{name}\",\"input\":{input}}}]}}}}\n\
         {{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{stamp}\",\"is_error\":true,\"content\":\"String to replace not found in file.\"}}]}}}}"
    )
}

/// A `Bash` call whose result came back an error.
fn failed_bash(command: &str, cwd: &str, stamp: &str) -> String {
    let input = serde_json::json!({ "command": command });
    format!(
        "{{\"type\":\"assistant\",\"cwd\":\"{cwd}\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{stamp}\",\"name\":\"Bash\",\"input\":{input}}}]}}}}\n\
         {{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{stamp}\",\"is_error\":true,\"content\":\"Exit code 1\"}}]}}}}"
    )
}

/// A `Bash` call line **and the result that came back**, with the directory it
/// ran in.
///
/// `cwd` is a top-level field of the transcript line rather than part of the
/// tool input, and the command is a JSON string — both are why the miner parses
/// these lines instead of scanning them for a needle.
///
/// ⚠ **The result is not optional decoration.** A call whose outcome is
/// unrecorded cannot have its `&&` confirmed — `a && b` only certainly runs `b`
/// when the call is known to have exited 0 — so a fixture without one silently
/// tests the rarest state there is (12 calls in the whole corpus) instead of the
/// ordinary one. Both lines are returned together because the caller joins them
/// with newlines anyway.
fn bash(command: &str, cwd: &str, stamp: &str) -> String {
    let input = serde_json::json!({ "command": command });
    // The timestamp is unique per call in these fixtures, so it serves as the id
    // joining the call to its answer.
    format!(
        "{{\"type\":\"assistant\",\"cwd\":\"{cwd}\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{stamp}\",\"name\":\"Bash\",\"input\":{input}}}]}}}}\n\
         {{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{stamp}\",\"is_error\":false,\"content\":\"…\"}}]}}}}"
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
    mine_corpus(transcripts, delegated, registry)
}

/// The same mine, named for the tests that care about the corpus directory —
/// `/mem`, which sits outside the `/code` root exactly as the real one does.
fn mine_corpus(
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
        "/mem",
        "/home/example",
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
fn the_path_below_the_project_is_kept_so_a_subtree_can_be_asked_about() {
    // The project counters answer "which repo" and stop there, which is why
    // "who built the Dhall reconciler" was unanswerable from them: every file in
    // xinutec-infra/plan/ is filed under `xinutec-infra`, indistinguishable from
    // a firewall tweak. The path below the project is what makes a subtree — or
    // a file extension — a thing you can ask about.
    let agents = mine(
        &[(
            "s1",
            vec![
                call(
                    Tool::Write,
                    "/code/xinutec-infra/plan/backup.dhall",
                    "2026-07-01T10:00:00Z",
                ),
                call(
                    Tool::Edit,
                    "/code/xinutec-infra/plan/backup.dhall",
                    "2026-07-01T10:01:00Z",
                ),
                call(
                    Tool::Read,
                    "/code/xinutec-infra/plan/backup.dhall",
                    "2026-07-01T10:02:00Z",
                ),
            ],
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"infra"}"#)],
    );

    let use_ = agents[0]
        .paths
        .get("xinutec-infra/plan/backup.dhall")
        .expect("the path is kept whole, project prefix included");
    assert_eq!((use_.edits, use_.reads), (2, 1));
    // The project counters still say what they always said.
    assert_eq!(agents[0].writes.get("xinutec-infra"), Some(&2));
}

#[test]
fn a_memory_is_not_indexed_as_a_working_path() {
    // The corpus sits outside the code root and is counted per memory; letting
    // it in here would list the file every session opens as work anyone does.
    let agents = mine(
        &[(
            "s1",
            vec![call(
                Tool::Edit,
                "/mem/project_alpha.md",
                "2026-07-01T10:00:00Z",
            )],
        )],
        &[],
    );

    assert!(agents[0].paths.is_empty(), "{:?}", agents[0].paths);
    assert_eq!(
        agents[0].memories.get("project_alpha").map(|u| u.edits),
        Some(1)
    );
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
fn the_name_a_session_goes_by_beats_the_one_the_registry_made_up() {
    // ⚠ Real shapes, both of them. The registry stopped holding chosen names and
    // now carries the CLI's own handle for a session — `code-c4`, the working
    // directory and two hex digits — while the name somebody picked is appended
    // to the transcript as the session goes along. Trusting the registry first
    // renamed every conversation on the page to a placeholder.
    let lines = vec![
        call(Tool::Write, "/code/thing/x", "2026-07-01T10:00:00Z"),
        r#"{"type":"agent-name","agentName":"health","sessionId":"s1"}"#.to_string(),
    ];
    let agents = mine(
        &[("s1", lines)],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"code-c4"}"#)],
    );

    assert_eq!(agents[0].name, "health");
}

#[test]
fn a_rename_wins_because_the_last_line_is_the_one_read() {
    // The property the registry was trusted for in the first place. These lines
    // are appended as the session goes, so the newest is what it goes by — and
    // a session named twice must not answer to its first name.
    let lines = vec![
        r#"{"type":"agent-name","agentName":"first","sessionId":"s1"}"#.to_string(),
        call(Tool::Write, "/code/thing/x", "2026-07-01T10:00:00Z"),
        r#"{"type":"agent-name","agentName":"second","sessionId":"s1"}"#.to_string(),
    ];
    let agents = mine(&[("s1", lines)], &[]);

    assert_eq!(agents[0].name, "second");
}

#[test]
fn a_name_line_belonging_to_another_session_names_nobody() {
    // A transcript prints other sessions' lines — this one does, in the output of
    // the very command that found the bug. The id on the line is what settles
    // whose name it is, and a line about somebody else falls through to the
    // fallbacks rather than renaming this agent.
    let lines = vec![
        call(Tool::Write, "/code/thing/x", "2026-07-01T10:00:00Z"),
        r#"{"type":"agent-name","agentName":"elsewhere","sessionId":"s2"}"#.to_string(),
    ];
    let agents = mine(
        &[("s1", lines)],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"code-c4"}"#)],
    );

    assert_eq!(agents[0].name, "code-c4");
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
fn an_agent_records_the_session_ids_filed_under_it_but_not_its_subagents() {
    // The join to the corpus. Every memory records the `originSessionId` that
    // wrote it, which stays a raw uuid unless the roster can say whose it was.
    // A dispatched transcript must NOT contribute an id of its own: its work is
    // already counted here, and a subagent never writes a memory under its own
    // identity, so an extra id could only ever resolve to the wrong thing.
    let own = vec![call(Tool::Write, "/code/thing/x", "2026-07-01T10:00:00Z")];
    let dispatched = vec![call(Tool::Write, "/code/thing/y", "2026-07-01T10:00:00Z")];
    let agents = mine_with(
        &[("s1", own)],
        &[("s1/subagents/agent-a.jsonl", dispatched)],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"builder"}"#)],
    );

    assert_eq!(agents[0].name, "builder");
    let ids: Vec<&str> = agents[0].sessions.iter().map(String::as_str).collect();
    assert_eq!(ids, ["s1"]);
}

/// A roster from mined agents, for the query tests.
fn roster_of(agents: Vec<Agent>) -> Agents {
    Agents {
        doing: Default::default(),
        effects: Default::default(),
        renames: Default::default(),
        memory_days: Default::default(),
        generated: "2026-08-01T00:00:00Z".into(),
        commits: 0,
        unattributed: 0,
        agents,
    }
}

#[test]
fn who_works_on_ranks_by_changes_and_shows_the_files_behind_the_number() {
    let heavy = vec![
        call(
            Tool::Write,
            "/code/infra/plan/a.dhall",
            "2026-07-01T10:00:00Z",
        ),
        call(
            Tool::Edit,
            "/code/infra/plan/a.dhall",
            "2026-07-01T10:01:00Z",
        ),
        call(
            Tool::Edit,
            "/code/infra/plan/b.dhall",
            "2026-07-01T10:02:00Z",
        ),
    ];
    // Reads the same tree constantly and changes it once: consulting is not
    // owning, so this must not outrank the one doing the work.
    let onlooker = vec![
        call(
            Tool::Read,
            "/code/infra/plan/a.dhall",
            "2026-07-01T10:00:00Z",
        ),
        call(
            Tool::Read,
            "/code/infra/plan/a.dhall",
            "2026-07-01T10:01:00Z",
        ),
        call(
            Tool::Read,
            "/code/infra/plan/b.dhall",
            "2026-07-01T10:02:00Z",
        ),
        call(
            Tool::Read,
            "/code/infra/plan/b.dhall",
            "2026-07-01T10:03:00Z",
        ),
        call(
            Tool::Write,
            "/code/infra/plan/b.dhall",
            "2026-07-01T10:04:00Z",
        ),
    ];
    // Busy elsewhere entirely — must not appear at all, not appear as a zero.
    let elsewhere = vec![call(
        Tool::Write,
        "/code/health/src/x.rs",
        "2026-07-01T10:00:00Z",
    )];
    let roster = roster_of(mine(
        &[("s1", heavy), ("s2", onlooker), ("s3", elsewhere)],
        &[
            ("100", r#"{"pid":100,"sessionId":"s1","name":"builder"}"#),
            ("101", r#"{"pid":101,"sessionId":"s2","name":"reader"}"#),
            ("102", r#"{"pid":102,"sessionId":"s3","name":"other"}"#),
        ],
    ));

    let found = roster.who_works_on("dhall");
    assert_eq!(
        found.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["builder", "reader"],
    );
    assert_eq!((found[0].edits, found[0].reads), (3, 0));
    assert_eq!((found[1].edits, found[1].reads), (1, 4));
    // The evidence, heaviest first, so the ranking can be checked rather than
    // believed.
    assert_eq!(found[0].files[0].path, "infra/plan/a.dhall");
    assert_eq!(found[0].files[0].edits, 2);
}

#[test]
fn who_works_on_matches_a_directory_and_an_extension_alike() {
    // "dhall" is a directory in the manifest model and a suffix in the
    // reconciler. Both are the same question, and one substring answers it.
    let roster = roster_of(mine(
        &[(
            "s1",
            vec![
                call(
                    Tool::Write,
                    "/code/pippijn/code/kubes/dhall/apps/home.dhall",
                    "2026-07-01T10:00:00Z",
                ),
                call(
                    Tool::Write,
                    "/code/infra/plan/wire.dhall",
                    "2026-07-01T10:01:00Z",
                ),
                call(
                    Tool::Write,
                    "/code/infra/plan/run.py",
                    "2026-07-01T10:02:00Z",
                ),
            ],
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"builder"}"#)],
    ));

    let found = roster.who_works_on("DHALL");
    let files: Vec<&str> = found[0].files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(files.len(), 2, "{files:?}");
    assert!(files.contains(&"infra/plan/wire.dhall"), "{files:?}");
    assert!(!files.iter().any(|f| f.ends_with(".py")), "{files:?}");
}

#[test]
fn an_empty_query_asks_nothing_and_is_answered_with_nothing() {
    // Not "everyone": a blank box must not render the entire roster as though it
    // were a result.
    let roster = roster_of(mine(
        &[(
            "s1",
            vec![call(
                Tool::Write,
                "/code/infra/a.dhall",
                "2026-07-01T10:00:00Z",
            )],
        )],
        &[],
    ));

    assert!(roster.who_works_on("").is_empty());
    assert!(roster.who_works_on("   ").is_empty());
}

#[test]
fn a_session_resolves_to_its_agent_and_a_forgotten_one_to_nobody() {
    let lines = vec![call(Tool::Write, "/code/thing/x", "2026-07-01T10:00:00Z")];
    let roster = Agents {
        doing: Default::default(),
        effects: Default::default(),
        renames: Default::default(),
        memory_days: Default::default(),
        generated: "2026-07-31T00:00:00Z".into(),
        commits: 0,
        unattributed: 0,
        agents: mine(
            &[("s1", lines)],
            &[("100", r#"{"pid":100,"sessionId":"s1","name":"builder"}"#)],
        ),
    };

    assert_eq!(roster.name_of_session("s1"), Some("builder"));
    // Claude Code prunes its own old sessions, so a memory outlives the
    // transcript that wrote it. That is an ordinary answer, not a failure —
    // the alternative is attributing it to whoever happens to sort first.
    assert_eq!(roster.name_of_session("s-pruned"), None);
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
fn opening_a_memory_is_attributed_to_that_memory_not_to_a_project() {
    // The corpus sits outside the code root, so `project_of` drops it — which is
    // right, since every session reads memories and that distinguishes nobody.
    // WHICH memory it opens distinguishes it a great deal, so that is kept.
    let agents = mine_corpus(
        &[(
            "s1",
            vec![
                call(Tool::Read, "/mem/project_recall.md", "2026-07-30T10:00:00Z"),
                call(Tool::Read, "/mem/project_recall.md", "2026-07-30T10:01:00Z"),
                call(Tool::Edit, "/mem/project_recall.md", "2026-07-30T10:02:00Z"),
                call(Tool::Write, "/code/thing/a", "2026-07-30T10:03:00Z"),
            ],
        )],
        &[],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"boss"}"#)],
    );

    let a = &agents[0];
    assert_eq!(a.memories.get("project_recall").map(|u| u.reads), Some(2));
    assert_eq!(a.memories.get("project_recall").map(|u| u.edits), Some(1));
    // ...and it is not also counted as a project, or "mem" would appear beside
    // the repositories as though it were one.
    assert_eq!(a.writes.get("thing"), Some(&1));
    assert_eq!(a.reads.len(), 0);
}

#[test]
fn the_index_is_not_a_memory_anyone_knows() {
    // MEMORY.md is given to every session, so counting opens of it would say the
    // same thing about everyone, which is nothing.
    let agents = mine_corpus(
        &[(
            "s1",
            vec![
                call(Tool::Read, "/mem/MEMORY.md", "2026-07-30T10:00:00Z"),
                call(Tool::Read, "/mem/notes/nested.md", "2026-07-30T10:01:00Z"),
                call(Tool::Read, "/mem/project_real.md", "2026-07-30T10:02:00Z"),
            ],
        )],
        &[],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"boss"}"#)],
    );

    let names: Vec<&str> = agents[0].memories.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["project_real"], "{names:?}");
}

#[test]
fn a_memory_a_subagent_opened_belongs_to_the_agent_that_sent_it() {
    // Same rule as the file counts: the subagent opened it because a named
    // session asked it to, so what it consulted is that session's knowledge.
    let agents = mine_corpus(
        &[(
            "s1",
            vec![call(Tool::Read, "/code/thing/a", "2026-07-30T10:00:00Z")],
        )],
        &[(
            "s1/subagents/agent-aaa.jsonl",
            vec![call(
                Tool::Read,
                "/mem/feedback_no_coauthor.md",
                "2026-07-30T10:05:00Z",
            )],
        )],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"boss"}"#)],
    );

    assert_eq!(agents.len(), 1);
    assert_eq!(
        agents[0]
            .memories
            .get("feedback_no_coauthor")
            .map(|u| u.reads),
        Some(1)
    );
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

#[test]
fn a_file_changed_from_the_shell_is_counted_as_its_own_dimension() {
    // The whole point of reading Bash: `Write` and `Edit` see none of this, so
    // the agent doing its editing through the shell loses the work.
    let agents = mine(
        &[(
            "s1",
            vec![
                bash(
                    "sed -i '' 's/a/b/' src/geo/osm.ts",
                    "/code/health",
                    "2026-07-01T10:00:00Z",
                ),
                bash(
                    "grep -rn hsmm src/geo/velocity.ts",
                    "/code/health",
                    "2026-07-01T10:01:00Z",
                ),
                call(
                    Tool::Edit,
                    "/code/health/src/geo/osm.ts",
                    "2026-07-01T10:02:00Z",
                ),
            ],
        )],
        &[("100", r#"{"sessionId":"s1","name":"health"}"#)],
    );
    let health = &agents[0];
    // Shell use is kept apart from tool use, so nothing that was already on the
    // page moves. The union happens at query time, and only there.
    assert_eq!(health.paths["health/src/geo/osm.ts"].edits, 1);
    assert_eq!(health.shell_paths["health/src/geo/osm.ts"].edits, 1);
    assert_eq!(health.shell_paths["health/src/geo/velocity.ts"].reads, 1);
    assert!(!health.paths.contains_key("health/src/geo/velocity.ts"));

    // ...and the query sums them, reporting the shell's share separately so the
    // evidence can be checked rather than believed.
    let roster = Agents {
        doing: Default::default(),
        effects: Default::default(),
        renames: Default::default(),
        memory_days: Default::default(),
        generated: String::new(),
        commits: 0,
        unattributed: 0,
        agents: agents.clone(),
    };
    let found = roster.who_works_on("osm.ts");
    assert_eq!(found[0].edits, 2);
    assert_eq!(found[0].files[0].edits, 2);
    assert_eq!(found[0].files[0].shell_edits, 1);
}

#[test]
fn build_output_is_not_work_anyone_is_credited_with() {
    // `rm -rf dist` changes a great many files and says nothing about who owns
    // the code that built them. Generated paths are 0.1% of tool-call use and
    // 4.3% of shell use on the live corpus, which is why this matters here and
    // barely showed before.
    let agents = mine(
        &[(
            "s1",
            vec![
                bash(
                    "rm -rf frontend/dist/main.js && cp x.ts src/a.ts",
                    "/code/health",
                    "2026-07-01T10:00:00Z",
                ),
                bash(
                    "cat frontend/logs/build.log node_modules/x/index.js",
                    "/code/health",
                    "2026-07-01T10:01:00Z",
                ),
            ],
        )],
        &[("100", r#"{"sessionId":"s1","name":"health"}"#)],
    );
    let paths: Vec<&String> = agents[0].shell_paths.keys().collect();
    assert_eq!(paths, ["health/src/a.ts", "health/x.ts"]);
}

/// A throwaway git repository with one commit, and the hash it produced.
fn one_commit(dir: &std::path::Path, file: &str, body: &str) -> String {
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            // `-C` sets the directory; it does NOT clear the inherited GIT_*
            // environment, and GIT_DIR/GIT_INDEX_FILE win over it. A commit made
            // with `git commit -a` or with explicit paths exports GIT_INDEX_FILE
            // for its hooks, so this suite — run by the pre-commit gate — built
            // its scratch repositories against the *committing* repository and
            // failed two tests that pass on their own. Cleared here rather than
            // in the gate, because a test must not care how it was invoked.
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_COMMON_DIR")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            // **Fixed dates, or the hash differs every run.** It did, and the
            // test flaked in CI about one run in thirty: a short hash of seven
            // digits and no letter is refused on purpose (see
            // `commits::hash_candidates`), so a hash that came out all-digit
            // attributed nothing. Pinning the dates pins the hash.
            .env("GIT_AUTHOR_DATE", "2026-07-01T10:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-07-01T10:00:00Z")
            .output()
            .expect("git runs in the devshell")
    };
    std::fs::create_dir_all(dir).unwrap();
    run(&["init", "-q"]);
    std::fs::write(dir.join(file), body).unwrap();
    run(&["add", file]);
    run(&["commit", "-qm", "one"]);
    String::from_utf8(run(&["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn a_commit_belongs_to_whoever_saw_its_hash_first() {
    // Every commit here has the same git author, so the repository cannot say
    // who wrote it. The hash does not exist until the commit is made, which
    // makes the earliest mention the author and every later one a quotation.
    //
    // Both wrong versions of this rule looked fine. Matching nine characters
    // found almost nothing, because `git commit` prints seven. Matching *any*
    // mention put five agents on one commit — including the session that was
    // only reading the history that afternoon, which is what this test pins.
    let dir = std::env::temp_dir().join(format!("commits-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let code = dir.join("code");
    let sha = one_commit(&code.join("demo"), "a.rs", "one\ntwo\nthree\n");
    let short = &sha[..7];

    let projects = dir.join("projects/-code");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let mention = |stamp: &str| {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"[main {short}] one\"}}]}}}}"
        )
    };
    // The reader quotes the hash later, and at greater length.
    std::fs::write(projects.join("s1.jsonl"), mention("2026-07-01T12:00:00Z")).unwrap();
    std::fs::write(projects.join("s2.jsonl"), mention("2026-07-01T10:00:00Z")).unwrap();
    for (pid, id, name) in [("1", "s1", "reader"), ("2", "s2", "author")] {
        std::fs::write(
            sessions.join(format!("{pid}.json")),
            format!(r#"{{"sessionId":"{id}","name":"{name}"}}"#),
        )
        .unwrap();
    }

    let found = scan(
        &dir.join("projects"),
        &sessions,
        code.to_str().unwrap(),
        "/mem",
        "/home/example",
        "2026-07-31T00:00:00Z",
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(found.commits, 1);
    assert_eq!(found.unattributed, 0);
    let author = found.agents.iter().find(|a| a.name == "author").unwrap();
    let reader = found.agents.iter().find(|a| a.name == "reader").unwrap();
    assert_eq!(author.commits, 1);
    assert_eq!(author.commit_lines["demo/a.rs"].added, 3);
    // The reader mentioned the same hash and is credited with nothing.
    assert_eq!(reader.commits, 0);
    assert!(reader.commit_lines.is_empty());
}

#[test]
fn work_on_another_machine_is_kept_under_that_machine() {
    // Where an agent's work lands when it is not on this one. It cannot go in
    // `paths`: `/etc/nixos/flake.nix` exists on odin and not here, and merging
    // the two would make every local answer wrong.
    let agents = mine(
        &[(
            "s1",
            vec![
                bash(
                    "ssh root@odin.xinutec.org 'cd /etc/nixos && sed -i s/a/b/ flake.nix'",
                    "/code/health",
                    "2026-07-01T10:00:00Z",
                ),
                bash(
                    "ssh odin 'cat /etc/nixos/machines/odin/drill.nix'",
                    "/code/health",
                    "2026-07-01T10:01:00Z",
                ),
                // Scratch and logs are not work, there as much as here.
                bash(
                    "ssh odin 'cat /tmp/out.txt; cat /var/log/drill.log'",
                    "/code/health",
                    "2026-07-01T10:02:00Z",
                ),
            ],
        )],
        &[("100", r#"{"sessionId":"s1","name":"fleet"}"#)],
    );
    let fleet = &agents[0];
    assert!(fleet.paths.is_empty(), "nothing local: {:?}", fleet.paths);
    assert!(fleet.shell_paths.is_empty());
    let keys: Vec<&String> = fleet.remote_paths.keys().collect();
    assert_eq!(
        keys,
        [
            "odin:/etc/nixos/flake.nix",
            "odin:/etc/nixos/machines/odin/drill.nix"
        ]
    );
    assert_eq!(fleet.remote_paths["odin:/etc/nixos/flake.nix"].edits, 1);

    // ...and a query finds it, saying which machine it is on.
    let roster = roster_of(agents.clone());
    let found = roster.who_works_on("nixos");
    assert_eq!(found[0].name, "fleet");
    assert_eq!(found[0].hosts, ["odin"]);
    let remote = found[0]
        .files
        .iter()
        .find(|f| f.path == "/etc/nixos/flake.nix")
        .expect("the remote file is evidence like any other");
    assert_eq!(remote.host.as_deref(), Some("odin"));
    assert_eq!(remote.edits, 1);
    // Remote use can only have come from a shell payload.
    assert_eq!(remote.shell_edits, 1);
}

/// A repository whose one file was written, then renamed, in two commits.
/// Returns both hashes, newest last.
fn commit_then_rename(dir: &std::path::Path, from: &str, to: &str) -> (String, String) {
    let run = |args: &[&str]| {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            // `-C` sets the directory; it does NOT clear the inherited GIT_*
            // environment, and GIT_DIR/GIT_INDEX_FILE win over it. A commit made
            // with `git commit -a` or with explicit paths exports GIT_INDEX_FILE
            // for its hooks, so this suite — run by the pre-commit gate — built
            // its scratch repositories against the *committing* repository and
            // failed two tests that pass on their own. Cleared here rather than
            // in the gate, because a test must not care how it was invoked.
            .env_remove("GIT_DIR")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_OBJECT_DIRECTORY")
            .env_remove("GIT_COMMON_DIR")
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            // Fixed dates pin the hashes, for the reason `one_commit` explains.
            .env("GIT_AUTHOR_DATE", "2026-07-01T10:00:00Z")
            .env("GIT_COMMITTER_DATE", "2026-07-01T10:00:00Z")
            .output()
            .expect("git runs in the devshell")
    };
    let head = || {
        String::from_utf8(run(&["rev-parse", "HEAD"]).stdout)
            .unwrap()
            .trim()
            .to_string()
    };
    std::fs::create_dir_all(dir.join(from).parent().unwrap()).unwrap();
    run(&["init", "-q"]);
    std::fs::write(dir.join(from), "one\ntwo\nthree\n").unwrap();
    run(&["add", from]);
    run(&["commit", "-qm", "write"]);
    let first = head();
    run(&["mv", from, to]);
    run(&["commit", "-qm", "rename"]);
    (first, head())
}

#[test]
fn a_renamed_file_keeps_the_history_it_had_under_its_old_name() {
    // The failure this pins is silent: ask who works on the file and you are
    // told about the days since it was renamed, with no sign that the rest
    // exists. Git is the only one of the three dimensions that knows the two
    // names are one file, so its answer is applied to all of them.
    //
    // And a pure move must not read as writing the file: `git mv` of a
    // three-line file used to count three lines added and three deleted.
    let dir = std::env::temp_dir().join(format!("renames-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let code = dir.join("code");
    let (wrote, moved) = commit_then_rename(&code.join("demo"), "src/osm.ts", "src/overpass.ts");

    let projects = dir.join("projects/-code");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    // One session: it edited the file under its old name, then made both
    // commits, so every dimension has evidence under both names.
    let line = |stamp: &str, text: &str| {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"{text}\"}}]}}}}"
        )
    };
    // The path has to sit under the temporary code root, or the miner drops it
    // as work outside the fleet — which is exactly what it should do.
    let edit = format!(
        "{{\"type\":\"assistant\",\"timestamp\":\"2026-07-01T09:00:00Z\",\"cwd\":\"{root}/demo\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"name\":\"Edit\",\"input\":{{\"replace_all\":false,\"file_path\":\"{root}/demo/src/osm.ts\"}}}}]}}}}",
        root = code.display()
    );
    std::fs::write(
        projects.join("s1.jsonl"),
        format!(
            "{edit}\n{}\n{}\n",
            line(
                "2026-07-01T10:00:00Z",
                &format!("[main {}] write", &wrote[..7])
            ),
            line(
                "2026-07-01T11:00:00Z",
                &format!("[main {}] rename", &moved[..7])
            ),
        ),
    )
    .unwrap();
    std::fs::write(
        sessions.join("1.json"),
        r#"{"sessionId":"s1","name":"geo"}"#,
    )
    .unwrap();

    let found = scan(
        &dir.join("projects"),
        &sessions,
        code.to_str().unwrap(),
        "/mem",
        "/home/example",
        "2026-07-31T00:00:00Z",
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(
        found.renames.get("demo/src/osm.ts"),
        Some(&"demo/src/overpass.ts".to_string()),
        "git knows the two names are one file"
    );
    let agent = &found.agents[0];
    // The tool edit was filed under the old name and now belongs to the new one.
    assert!(!agent.paths.contains_key("demo/src/osm.ts"));
    assert_eq!(agent.paths["demo/src/overpass.ts"].edits, 1);
    // Three lines written once — not written twice and deleted once, which is
    // what a rename read as a delete-plus-add came to.
    let lines = &agent.commit_lines["demo/src/overpass.ts"];
    assert_eq!((lines.added, lines.deleted), (3, 0));

    // Asked about the name it no longer has, the file still answers.
    let old = found.who_works_on("osm.ts");
    assert_eq!(old.len(), 1, "the old name finds the work");
    assert_eq!(old[0].files[0].path, "demo/src/overpass.ts");
    assert_eq!(old[0].files[0].was, ["demo/src/osm.ts"]);
    assert_eq!(found.who_works_on("overpass").len(), 1);
}

#[test]
fn the_timeline_records_what_was_done_and_how_it_turned_out() {
    // The verdict comes from a LATER line: the result names the call's id, so a
    // row is written unresolved and filled in when the answer arrives. A
    // corpus of 90,166 Bash calls has 12 that never got one.
    let dir = std::env::temp_dir().join(format!("doing-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let projects = dir.join("projects/-code");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let call = |id: &str, cmd: &str, stamp: &str| {
        let input = serde_json::json!({ "command": cmd });
        format!(
            "{{\"type\":\"assistant\",\"cwd\":\"/code/health\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"Bash\",\"input\":{input}}}]}}}}"
        )
    };
    let result = |id: &str, failed: bool| {
        format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\",\"is_error\":{failed},\"content\":\"…\"}}]}}}}"
        )
    };
    std::fs::write(
        projects.join("s1.jsonl"),
        [
            call("t1", "cargo test -p geo", "2026-07-01T10:00:00Z"),
            result("t1", true),
            call(
                "t2",
                "sed -i '' 's/a/b/' src/geo/osm.ts",
                "2026-07-01T10:05:00Z",
            ),
            result("t2", false),
            // Navigation is not work and never reaches the timeline.
            call("t3", "cd src && ls", "2026-07-01T10:06:00Z"),
            result("t3", false),
        ]
        .join("\n"),
    )
    .unwrap();
    std::fs::write(
        sessions.join("1.json"),
        r#"{"sessionId":"s1","name":"geo"}"#,
    )
    .unwrap();

    let found = scan(
        &dir.join("projects"),
        &sessions,
        "/code",
        "/mem",
        "/home/example",
        "2026-07-31T00:00:00Z",
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let doing = &found.doing;
    let rows: Vec<(&str, &str, reader::doing::Verdict)> = doing
        .rows
        .iter()
        .map(|row| {
            (
                doing.kinds[row.k as usize].as_str(),
                doing.projects[row.p.expect("under a project") as usize].as_str(),
                row.v,
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("test", "health", reader::doing::Verdict::Failed),
            ("edit", "health", reader::doing::Verdict::Ok),
        ]
    );
    assert_eq!(doing.agents, ["geo"]);
    // Oldest first, so a reader walks it forwards.
    assert!(doing.rows[0].t < doing.rows[1].t);
}

#[test]
fn the_effects_say_which_file_and_which_command_did_it() {
    // ⚠ **The question a reader asks standing on a timeline row.** `doing.json`
    // says "this turn was editing, in health, and it worked"; this says which
    // files, and shows the command, so the claim can be checked rather than
    // taken. Keyed by `(agent, minute)` — the key a timeline row already has —
    // so opening a turn is a filter and no published format changes.
    let dir = std::env::temp_dir().join(format!("effects-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let projects = dir.join("projects/-code");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let call = |id: &str, cmd: &str, stamp: &str| {
        let input = serde_json::json!({ "command": cmd });
        format!(
            "{{\"type\":\"assistant\",\"cwd\":\"/code/health\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"Bash\",\"input\":{input}}}]}}}}"
        )
    };
    let result = |id: &str| {
        format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\",\"is_error\":false,\"content\":\"…\"}}]}}}}"
        )
    };
    std::fs::write(
        projects.join("s1.jsonl"),
        [
            call(
                "t1",
                "sed -i '' 's/a/b/' src/osm.ts",
                "2026-07-01T10:00:00Z",
            ),
            result("t1"),
            call(
                "t2",
                "grep -rn hsmm src/velocity.ts",
                "2026-07-01T10:01:00Z",
            ),
            result("t2"),
            // ⚠ Two admissions, which have to travel or the artefact reads as a
            // complete account of the work: one subject a glob BOUNDS, and one
            // nothing bounds at all.
            call("t3", "wc -l \"$OUT/report.txt\"", "2026-07-01T10:02:00Z"),
            result("t3"),
            call(
                "t4",
                "for f in logs/*.log; do wc -l \"$f\"; done",
                "2026-07-01T10:03:00Z",
            ),
            result("t4"),
        ]
        .join("\n"),
    )
    .unwrap();
    std::fs::write(
        sessions.join("1.json"),
        r#"{"sessionId":"s1","name":"geo"}"#,
    )
    .unwrap();

    let found = scan(
        &dir.join("projects"),
        &sessions,
        "/code",
        "/mem",
        "/home/example",
        "2026-07-31T00:00:00Z",
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    let effects = &found.effects;
    let rows: Vec<(reader::effects::Did, Option<&str>, Option<&str>, &str)> = effects
        .rows
        .iter()
        .map(|row| {
            (
                row.k,
                row.p.map(|at| effects.paths[at as usize].as_str()),
                row.q.map(|at| effects.patterns[at as usize].as_str()),
                effects.commands[row.c as usize].as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            (
                reader::effects::Did::Wrote,
                Some("/code/health/src/osm.ts"),
                None,
                "sed -i  s/a/b/ src/osm.ts",
            ),
            (
                reader::effects::Did::Searched,
                Some("/code/health/src/velocity.ts"),
                Some("hsmm"),
                "grep -rn hsmm src/velocity.ts",
            ),
            // Nothing determines `$OUT`, and nothing bounds it either — so the
            // row carries no path at all, and still exists.
            (
                reader::effects::Did::Unnamed,
                None,
                None,
                "wc -l $OUT/report.txt",
            ),
            // A glob DOES bound it: the row carries the pattern the subject is a
            // subset of, which is a different fact from the one above.
            (
                reader::effects::Did::Unnamed,
                Some("/code/health/logs/*.log"),
                None,
                "wc -l $f",
            ),
        ]
    );
    // The verdict arrives on a later line and is filled in afterwards, as in the
    // timeline — a row written before its result is not a row without one.
    assert!(
        effects
            .rows
            .iter()
            .all(|row| row.v == reader::doing::Verdict::Ok)
    );
    assert_eq!(effects.agents, ["geo"]);
    // Oldest first, so a reader walks it forwards.
    assert!(effects.rows.windows(2).all(|w| w[0].t <= w[1].t));
    // ⚠ **The command is a dictionary index, not a string per row.** That is
    // where the artefact's size went — measured over the real corpus, the
    // commands that bear an effect are 9.7 MB against 41.5 MB for every distinct
    // whole call.
    assert!(effects.commands.len() <= effects.rows.len());
}

#[test]
fn a_call_the_user_refused_names_no_files() {
    // ⚠ **A refused call never ran**, so every path in it is an intention and
    // not an act. 76 such calls in the corpus name 105 file uses — 21 of them
    // *writes*, to files that nothing ever wrote.
    //
    // The refusal still reaches the timeline, with its own verdict. That an
    // agent tried and was told no is worth seeing; crediting it with the work
    // is not. Knowing a command did not run is knowledge, not absence.
    let dir = std::env::temp_dir().join(format!("refused-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let projects = dir.join("projects/-code");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&projects).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let call = |id: &str, cmd: &str, stamp: &str| {
        let input = serde_json::json!({ "command": cmd });
        format!(
            "{{\"type\":\"assistant\",\"cwd\":\"/code/health\",\"timestamp\":\"{stamp}\",\"message\":{{\"content\":[{{\"type\":\"tool_use\",\"id\":\"{id}\",\"name\":\"Bash\",\"input\":{input}}}]}}}}"
        )
    };
    let done = |id: &str| {
        format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\",\"is_error\":false,\"content\":\"…\"}}]}}}}"
        )
    };
    // The harness's own sentence, at the front of the content — the anchor is
    // what makes this safe to match, since the words also appear in the output
    // of any session that searched for them.
    let refused = |id: &str| {
        format!(
            "{{\"type\":\"user\",\"message\":{{\"content\":[{{\"type\":\"tool_result\",\"tool_use_id\":\"{id}\",\"is_error\":true,\"content\":\"The user doesn't want to proceed with this tool use. The tool use was rejected.\"}}]}}}}"
        )
    };
    std::fs::write(
        projects.join("s1.jsonl"),
        [
            call(
                "t1",
                "sed -i '' 's/a/b/' src/geo/kept.ts",
                "2026-07-01T10:00:00Z",
            ),
            done("t1"),
            call(
                "t2",
                "sed -i '' 's/a/b/' src/geo/never.ts",
                "2026-07-01T10:05:00Z",
            ),
            refused("t2"),
        ]
        .join("\n"),
    )
    .unwrap();
    std::fs::write(
        sessions.join("1.json"),
        r#"{"sessionId":"s1","name":"geo"}"#,
    )
    .unwrap();

    let found = scan(
        &dir.join("projects"),
        &sessions,
        "/code",
        "/mem",
        "/home/example",
        "2026-07-31T00:00:00Z",
    )
    .unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(found.who_works_on("kept.ts").len(), 1, "the call that ran");
    assert!(
        found.who_works_on("never.ts").is_empty(),
        "the refused call must not put a path in anyone's record"
    );
    // Refused is its own verdict, not a kind of failure: the two mean different
    // things about whether a process existed.
    let verdicts: Vec<reader::doing::Verdict> = found.doing.rows.iter().map(|row| row.v).collect();
    assert_eq!(
        verdicts,
        [reader::doing::Verdict::Ok, reader::doing::Verdict::Rejected]
    );
}

#[test]
fn a_tool_call_that_failed_changed_nothing() {
    // ⚠ **990 failed `Edit`s and 289 failed `Write`s in the live corpus, every
    // one counted as work.** An `Edit` fails when its `old_string` is not in the
    // file, which means the file was left exactly as it was — crediting an agent
    // with that change credits it with something that did not happen.
    //
    // A tool call settles this outright, unlike a shell script: it does one
    // thing, so its result is a fact about that thing rather than about a list
    // of them. No reachability reasoning applies.
    let agents = mine(
        &[(
            "s1",
            vec![
                call(
                    Tool::Edit,
                    "/code/health/src/kept.ts",
                    "2026-07-01T10:00:00Z",
                ),
                failed_call(
                    Tool::Edit,
                    "/code/health/src/never.ts",
                    "2026-07-01T10:01:00Z",
                ),
                failed_call(
                    Tool::Write,
                    "/code/health/src/nor.ts",
                    "2026-07-01T10:02:00Z",
                ),
                failed_call(
                    Tool::Read,
                    "/code/health/src/missing.ts",
                    "2026-07-01T10:03:00Z",
                ),
            ],
        )],
        &[("100", r#"{"sessionId":"s1","name":"health"}"#)],
    );
    let paths: Vec<&String> = agents[0].paths.keys().collect();
    assert_eq!(paths, ["health/src/kept.ts"]);
    assert_eq!(agents[0].writes.get("health"), Some(&1));
    // A `Read` that errored opened nothing either — the file was not there.
    assert_eq!(agents[0].reads.get("health"), None);
}

#[test]
fn a_use_the_outcome_cannot_confirm_is_possible_rather_than_lost() {
    // ⚠ **Neither counting it nor dropping it is true.** In `a && b` under a
    // call that failed, `b` may or may not have run: counting it claims work
    // that may never have happened, and dropping it denies work that may well
    // have. 19,256 file uses in the corpus are of this kind, and the record
    // holds the two claims apart rather than picking one.
    let agents = mine(
        &[(
            "s1",
            vec![failed_bash(
                "cat src/certain.ts && cat src/perhaps.ts",
                "/code/health",
                "2026-07-01T10:00:00Z",
            )],
        )],
        &[("100", r#"{"sessionId":"s1","name":"health"}"#)],
    );
    let shell = &agents[0].shell_paths;
    // The first command runs whatever happens, so it is not in doubt.
    let certain = &shell["health/src/certain.ts"];
    assert_eq!((certain.reads, certain.maybe_reads), (1, 0));
    // The second needed the first to have worked, and the call reports only
    // that something in it did not.
    let perhaps = &shell["health/src/perhaps.ts"];
    assert_eq!((perhaps.reads, perhaps.maybe_reads), (0, 1));
}

#[test]
fn a_session_that_only_titles_another_is_not_an_agent() {
    // ⚠ **These outnumbered the real agents four to one**: 307 of 324 rows on
    // the page were one-shot Haiku sessions the CLI spawns to name a
    // conversation, each a bare uuid with every counter at zero.
    //
    // Both halves of the test earn their place. Older CLI versions wrote the
    // title into the working session's own transcript, so nine real sessions —
    // 11,000 to 110,000 lines, thousands of tool calls — carry an `ai-title`
    // line too. Excluding on that line alone would delete the largest sessions
    // in the corpus.
    // The four that never got an `ai-title` line are known only by the prompt
    // they were handed.
    let untitled = "{\"type\":\"user\",\"timestamp\":\"2026-07-01T09:00:00Z\",\"message\":{\"content\":[{\"type\":\"text\",\"text\":\"Below is part of a conversation between a person and a coding agent, with the tool calls removed.\"}]}}".to_string();
    let titler = |id: &str| {
        format!(
            "{{\"type\":\"user\",\"timestamp\":\"2026-07-01T10:00:00Z\",\"message\":{{\"content\":[{{\"type\":\"text\",\"text\":\"Below is part of a conversation…\"}}]}}}}\n\
             {{\"type\":\"ai-title\",\"aiTitle\":\"Some title\",\"sessionId\":\"{id}\"}}"
        )
    };
    let agents = mine(
        &[
            (
                "s1",
                vec![call(
                    Tool::Edit,
                    "/code/health/a.ts",
                    "2026-07-01T10:00:00Z",
                )],
            ),
            ("s2", vec![titler("s2")]),
            ("s4", vec![untitled]),
            // A working session that also carries a title line stays.
            (
                "s3",
                vec![
                    titler("s3"),
                    call(Tool::Write, "/code/health/b.ts", "2026-07-01T11:00:00Z"),
                ],
            ),
        ],
        &[
            ("100", r#"{"sessionId":"s1","name":"health"}"#),
            ("101", r#"{"sessionId":"s3","name":"observe"}"#),
        ],
    );
    let names: Vec<&String> = agents.iter().map(|a| &a.name).collect();
    assert_eq!(
        names,
        ["health", "observe"],
        "the titler is not one of them"
    );
}

#[test]
fn a_memory_read_from_the_shell_is_attributed_to_that_memory() {
    // ⚠ **The shell half of the corpus was dropped entirely.** The tool-call
    // site has had a `memory_of` arm since it was written; the shell site had
    // only the code-root filter, and the corpus is outside the code root — so
    // `tail`, `grep`, `sed` and `cat >>` over a memory counted for nothing.
    //
    // It matters because there is no recall channel: every memory arrival is a
    // `Read` or a command, so searching the corpus by hand is one of only two
    // ways a DEMOTED memory is ever reached, and it was the half the evidence
    // could not see.
    let agents = mine_corpus(
        &[(
            "s1",
            vec![
                bash(
                    "tail -60 /mem/project_recall.md",
                    "/code/memview",
                    "2026-07-30T10:00:00Z",
                ),
                bash(
                    "sed -i '' 's/a/b/' /mem/project_recall.md",
                    "/code/memview",
                    "2026-07-30T10:01:00Z",
                ),
            ],
        )],
        &[],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"boss"}"#)],
    );

    let use_ = &agents[0].memories["project_recall"];
    assert_eq!(use_.reads, 1, "{:?}", agents[0].memories);
    assert_eq!(use_.edits, 1, "{:?}", agents[0].memories);
    // And it is not also charged to a project, or `mem` would stand beside the
    // repositories as though the corpus were one.
    assert!(!agents[0].shell_paths.keys().any(|p| p.contains("recall")));
}

#[test]
fn a_glob_over_the_corpus_names_no_memory() {
    // ⚠ **A flat corpus makes `*.md` collapse to a stem of `*`.** Counted, that
    // invented a memory called `*` with 459 uses — more than any real one has,
    // and top of any report that iterates the map.
    //
    // Dropped rather than expanded to everything the pattern matches: `grep -l`
    // over the corpus reads all of it, so expanding would give every memory the
    // same score and destroy the ranking this feeds.
    let agents = mine_corpus(
        &[(
            "s1",
            vec![
                bash(
                    "grep -l roadmap /mem/*.md",
                    "/code/memview",
                    "2026-07-30T10:00:00Z",
                ),
                bash(
                    "ls /mem/project_*.md",
                    "/code/memview",
                    "2026-07-30T10:01:00Z",
                ),
                // The real one beside them, so this cannot pass by counting
                // nothing at all.
                bash(
                    "cat /mem/project_real.md",
                    "/code/memview",
                    "2026-07-30T10:02:00Z",
                ),
            ],
        )],
        &[],
        &[("100", r#"{"pid":100,"sessionId":"s1","name":"boss"}"#)],
    );

    let names: Vec<&str> = agents[0].memories.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["project_real"], "{names:?}");
}

/// The episode boundary: which `user` line is somebody typing.
///
/// ⚠ **This is the one predicate whose failure MERGES two instructions**, and a
/// merge is unrecoverable downstream where a duplicate bracket is only noise.
/// Testing `tool_result` as a bare word cost 17 of them across the corpus.
mod is_prompt {
    use memview::agents::is_prompt;

    /// A tool result, as the transcript writes one.
    const RESULT: &[u8] = br#"{"type":"user","message":{"role":"user","content":[{"tool_use_id":"x","type":"tool_result","content":"ok"}]},"timestamp":"2026-08-17T10:00:00Z"}"#;

    #[test]
    fn a_tool_result_is_not_a_prompt() {
        assert!(!is_prompt(RESULT));
    }

    #[test]
    fn somebody_typing_is_a_prompt() {
        let string_content =
            br#"{"type":"user","message":{"role":"user","content":"proceed"},"timestamp":"t"}"#;
        assert!(is_prompt(string_content));
        let block_content = br#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"proceed"}]},"timestamp":"t"}"#;
        assert!(is_prompt(block_content));
    }

    #[test]
    fn a_prompt_that_talks_about_tool_results_is_still_a_prompt() {
        // The regression. Somebody asking about the machinery is not the
        // machinery — and inside a JSON string the quotes are escaped, so the
        // key this looks for cannot appear in prose.
        let asking = br#"{"type":"user","message":{"role":"user","content":"why does a tool_result line say \"type\":\"tool_result\" twice?"},"timestamp":"t"}"#;
        assert!(is_prompt(asking));
    }

    #[test]
    fn an_assistant_line_is_never_a_prompt() {
        let assistant =
            br#"{"type":"assistant","message":{"role":"assistant","content":"done"},"timestamp":"t"}"#;
        assert!(!is_prompt(assistant));
    }
}

// --- memory-days must survive a pruned transcript (#884's outcome) -----------

use memview::agents::{MemoryDays, carry_forward};

fn days_file(dir: &std::path::Path, body: &str) -> std::path::PathBuf {
    let p = dir.join("memory-days.json");
    std::fs::write(&p, body).expect("write");
    p
}

/// ⚠ The whole point: a day the previous file recorded survives a run whose
/// transcripts no longer show it, and is COUNTED so the pruning is visible.
#[test]
fn a_day_whose_transcript_is_gone_is_carried_and_counted() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = days_file(
        dir.path(),
        r#"{"reference_a":{"reads":[100,101],"edits":[100]}}"#,
    );
    let mut now: std::collections::BTreeMap<String, MemoryDays> = std::collections::BTreeMap::new();
    now.insert(
        "reference_a".to_string(),
        MemoryDays {
            reads: vec![101],
            edits: vec![],
        },
    );

    let carried = carry_forward(&path, &mut now).expect("parses");

    let got = &now["reference_a"];
    assert_eq!(
        got.reads,
        vec![100, 101],
        "day 100 was pruned away and must survive"
    );
    assert_eq!(got.edits, vec![100]);
    assert_eq!(
        carried, 2,
        "one read and one edit were only in the earlier record"
    );
}

/// A re-mine that sees everything carries nothing, so the count stays a real
/// signal rather than noise on every run.
#[test]
fn a_complete_remine_carries_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = days_file(dir.path(), r#"{"reference_a":{"reads":[100,101]}}"#);
    let mut now: std::collections::BTreeMap<String, MemoryDays> = std::collections::BTreeMap::new();
    now.insert(
        "reference_a".to_string(),
        MemoryDays {
            reads: vec![100, 101],
            edits: vec![],
        },
    );
    assert_eq!(carry_forward(&path, &mut now).expect("parses"), 0);
    assert_eq!(now["reference_a"].reads, vec![100, 101]);
}

/// A memory that has left the corpus entirely still keeps its history — #884
/// measures memories that were DEMOTED, so losing them would delete the study.
#[test]
fn a_memory_absent_from_this_run_keeps_its_days() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = days_file(dir.path(), r#"{"reference_gone":{"reads":[100]}}"#);
    let mut now: std::collections::BTreeMap<String, MemoryDays> = std::collections::BTreeMap::new();
    assert_eq!(carry_forward(&path, &mut now).expect("parses"), 1);
    assert_eq!(now["reference_gone"].reads, vec![100]);
}

/// A first run has no previous file, and that is not an error.
#[test]
fn a_missing_previous_file_is_the_first_run() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut now: std::collections::BTreeMap<String, MemoryDays> = std::collections::BTreeMap::new();
    assert_eq!(
        carry_forward(&dir.path().join("absent.json"), &mut now).expect("first run"),
        0
    );
}

/// ⚠ A file that EXISTS but will not parse must FAIL the mine. Returning 0 would
/// read as "nothing to carry" and the run would overwrite it, deleting the whole
/// record silently — which is the failure this function was written to prevent.
#[test]
fn a_corrupt_previous_file_fails_rather_than_being_overwritten() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut now: std::collections::BTreeMap<String, MemoryDays> = std::collections::BTreeMap::new();
    let bad = days_file(dir.path(), "{not json");
    assert!(carry_forward(&bad, &mut now).is_err());
}

/// ⚠ **A `generated` field nobody consults is decoration.** `agents.json` has
/// carried one all along and still produced three wrong analyses (#1210); the
/// fourth was a demotion argued from breadth figures for memories written after
/// the mine. These pin the refusal.
///
/// ⚠ **The transcripts are the witness, not the filesystem.** The first version
/// compared mtimes: 55 alarms for 2 real changes, because something had touched
/// fifty-two files without altering a word.
fn transcript(dir: &std::path::Path, name: &str, lines: &[&str]) -> std::path::PathBuf {
    let path = dir.join(format!("{name}.jsonl"));
    std::fs::write(&path, lines.join("\n")).unwrap();
    path
}

fn wrote(stamp: &str, memory: &str) -> String {
    format!(
        r#"{{"timestamp":"{stamp}","message":{{"content":[{{"name":"Write","input":{{"file_path":"/home/example/.claude/projects/-x/memory/{memory}.md"}}}}]}}}}"#
    )
}

#[test]
fn a_mine_that_has_seen_every_write_is_fresh() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "s1",
        &[&wrote("2026-08-01T10:00:00Z", "old_one")],
    );
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert!(!fresh.is_stale(), "{:?}", fresh.unseen);
}

#[test]
fn a_memory_written_after_the_mine_makes_it_stale_and_is_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "s1",
        &[
            &wrote("2026-08-01T10:00:00Z", "before"),
            &wrote("2026-08-27T09:52:56Z", "after"),
        ],
    );
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert!(fresh.is_stale());
    assert_eq!(fresh.unseen, vec!["after".to_string()]);
}

/// ⚠ **Editing the index is not staleness.** `MEMORY.md` lives in the corpus
/// directory, so the write scan sees it like any other file — but both readers
/// of `freshness` load the index live from disk, so a change to it cannot move
/// a figure that comes from the artefact. Counting it made the refusal fire
/// every time the memory session touched the root, and a refusal that fires on
/// a harmless change is one people learn to override.
#[test]
fn an_edit_to_the_index_does_not_make_the_mine_stale() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "s1",
        &[&wrote("2026-08-27T09:52:56Z", memview::agents::INDEX_STEM)],
    );
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert!(!fresh.is_stale(), "{:?}", fresh.unseen);
}

/// And a real memory written in the same breath still does.
#[test]
fn a_memory_written_beside_an_index_edit_is_still_named() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "s1",
        &[
            &wrote("2026-08-27T09:52:56Z", memview::agents::INDEX_STEM),
            &wrote("2026-08-27T09:53:10Z", "a_real_one"),
        ],
    );
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert_eq!(fresh.unseen, vec!["a_real_one".to_string()]);
}

// ⚠ A bare `> name.md` is NOT a memory, and asserting it was is how the first
// version of this shipped. The test that stood here fed
// `cat > written_by_heredoc.md` with no directory and expected a memory named
// `written_by_heredoc`; the reader resolves the path and correctly refuses it.
// Superseded by `a_heredoc_write_to_a_memory_is_seen`, which names a real corpus
// path, and by `a_markdown_file_outside_the_corpus_is_not_a_memory` (#1218).

/// ⚠ **Prose is not a path.** A transcript says things like "memory/preferences
/// cannot fulfil them"; a bare `/memory/` substring match invented
/// `preferences` as a memory. The tool argument is the anchor.
#[test]
fn prose_mentioning_memory_is_not_a_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let line = r#"{"timestamp":"2026-08-27T09:00:00Z","message":{"content":[{"text":"see memory/preferences cannot fulfill them"}]}}"#;
    transcript(dir.path(), "s1", &[line]);
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert!(!fresh.is_stale(), "{:?}", fresh.unseen);
}

/// ⚠ **Without this exclusion the check refuses ALWAYS.** Claude Code appends to
/// the running session's transcript as the tool runs, so it postdates every
/// artefact by construction — and a guard that always fires trains people to
/// pass the override, which is worse than no guard.
#[test]
fn the_running_sessions_own_transcript_does_not_make_the_mine_stale() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "abc123",
        &[&wrote("2026-08-27T10:00:00Z", "mine")],
    );
    let live = memview::agents::freshness(
        "2026-08-26T23:33:31Z",
        &[dir.path()],
        Some("abc123"),
        "/home/example",
    );
    assert!(!live.is_stale(), "{:?}", live.unseen);
    let other = memview::agents::freshness(
        "2026-08-26T23:33:31Z",
        &[dir.path()],
        Some("zzz999"),
        "/home/example",
    );
    assert!(other.is_stale());
}

fn bash_row(stamp: &str, command: &str) -> String {
    format!(
        r#"{{"timestamp":"{stamp}","cwd":"/home/example/Code","message":{{"content":[{{"name":"Bash","input":{{"command":{}}}}}]}}}}"#,
        serde_json::to_string(command).unwrap()
    )
}

/// ⚠ **`echo x > /tmp/note.md` is not a memory**, and the `> name.md` heuristic
/// this replaced counted it whenever the stem collided with one (#1218).
#[test]
fn a_markdown_file_outside_the_corpus_is_not_a_memory() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "s1",
        &[&bash_row(
            "2026-08-27T09:00:00Z",
            "echo hi > /tmp/feedback_tdd_first.md",
        )],
    );
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert!(!fresh.is_stale(), "{:?}", fresh.unseen);
}

/// A heredoc write to a real memory is what the tool-name scan missed, and is
/// why the shell is read at all.
#[test]
fn a_heredoc_write_to_a_memory_is_seen() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "s1",
        &[&bash_row(
            "2026-08-27T09:00:00Z",
            "cat > /home/example/.claude/projects/-x/memory/feedback_written.md <<'MD'\nbody\nMD",
        )],
    );
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert_eq!(fresh.unseen, vec!["feedback_written".to_string()]);
}

/// ⚠ **Reading a memory is not writing one.** The redirect heuristic could not
/// tell the two apart inside a compound command.
#[test]
fn reading_a_memory_does_not_make_the_mine_stale() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "s1",
        &[&bash_row(
            "2026-08-27T09:00:00Z",
            "cat /home/example/.claude/projects/-x/memory/feedback_read_only.md",
        )],
    );
    let fresh =
        memview::agents::freshness("2026-08-26T23:33:31Z", &[dir.path()], None, "/home/example");
    assert!(!fresh.is_stale(), "{:?}", fresh.unseen);
}

/// ⚠ **The accumulator that decides commit authorship is order-INDEPENDENT**,
/// and that is what makes a resumed or split scan possible: it keeps the
/// earliest TIMESTAMP, not the first transcript walked. #1240 recorded the
/// opposite, on a reading of the call site rather than of the rule.
#[test]
fn commit_authorship_merges_to_the_same_answer_in_either_order() {
    use memview::agents::{FirstSeen, keep_earliest};

    let early: FirstSeen = [(
        "abc123".to_string(),
        ("2026-08-01T10:00:00Z".to_string(), "memview".to_string()),
    )]
    .into_iter()
    .collect();
    let late: FirstSeen = [(
        "abc123".to_string(),
        ("2026-08-05T10:00:00Z".to_string(), "recall".to_string()),
    )]
    .into_iter()
    .collect();

    let mut one = early.clone();
    keep_earliest(&mut one, late.clone());
    let mut other = late;
    keep_earliest(&mut other, early);

    assert_eq!(
        one, other,
        "the merge must not depend on which half arrived first"
    );
    assert_eq!(
        one["abc123"].1, "memview",
        "the earliest sighting owns the commit"
    );
}

/// A hash only one half saw is carried, not dropped — otherwise a resumed scan
/// would lose every commit whose only mention sits in the part it skipped.
#[test]
fn a_sighting_only_one_half_has_survives_the_merge() {
    use memview::agents::{FirstSeen, keep_earliest};

    let mut into: FirstSeen = [(
        "aaa".to_string(),
        ("2026-08-01T00:00:00Z".to_string(), "memview".to_string()),
    )]
    .into_iter()
    .collect();
    let other: FirstSeen = [(
        "bbb".to_string(),
        ("2026-08-02T00:00:00Z".to_string(), "recall".to_string()),
    )]
    .into_iter()
    .collect();
    keep_earliest(&mut into, other);
    assert_eq!(into.len(), 2);
    assert_eq!(into["bbb"].1, "recall");
}
