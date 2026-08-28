//! Turning a lint error into a task somebody is actually addressed by (#1235).

use std::path::Path;

use memview::blame::{MARKER, attribute, open_task_in, subject, writes};

fn transcript(dir: &Path, session: &str, lines: &[&str]) {
    std::fs::write(dir.join(format!("{session}.jsonl")), lines.join("\n"))
        .expect("write transcript");
}

fn tool(stamp: &str, name: &str, path: &str) -> String {
    format!(
        r#"{{"timestamp":"{stamp}","message":{{"content":[{{"type":"tool_use","name":"{name}","input":{{"file_path":"{path}"}}}}]}}}}"#
    )
}

#[test]
fn a_write_names_the_session_that_made_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "sess-a",
        &[&tool(
            "2026-08-28T09:00:00Z",
            "Write",
            "/home/example/.claude/projects/-x/memory/feedback_new.md",
        )],
    );
    let found = attribute(
        dir.path(),
        &["feedback_new.md".to_string()],
        "/home/example",
    );
    assert_eq!(found["feedback_new.md"].session, "sess-a");
}

/// ⚠ **A mention is not a write.** The session diagnosing a lint failure names
/// every failing memory while doing so; attributing on the name alone would
/// blame the reader and leave the author untouched.
#[test]
fn merely_naming_a_memory_does_not_claim_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    transcript(
        dir.path(),
        "sess-reader",
        &[&tool(
            "2026-08-28T09:00:00Z",
            "Read",
            "/home/example/.claude/projects/-x/memory/feedback_new.md",
        )],
    );
    let found = attribute(
        dir.path(),
        &["feedback_new.md".to_string()],
        "/home/example",
    );
    assert!(found.is_empty(), "{found:?}");
}

/// The earliest write is the creation; later ones are edits, and the question
/// this answers is who wrote it FIRST.
#[test]
fn the_earliest_writer_owns_it_not_the_latest() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = "/home/example/.claude/projects/-x/memory/feedback_new.md";
    transcript(
        dir.path(),
        "sess-late",
        &[&tool("2026-08-28T12:00:00Z", "Edit", path)],
    );
    transcript(
        dir.path(),
        "sess-first",
        &[&tool("2026-08-28T09:00:00Z", "Write", path)],
    );
    let found = attribute(
        dir.path(),
        &["feedback_new.md".to_string()],
        "/home/example",
    );
    assert_eq!(found["feedback_new.md"].session, "sess-first");
}

/// ⚠ The whole reason this class exists: a heredoc write skips the stamping
/// path, so the memory carries no `originSessionId` and the frontmatter cannot
/// say who wrote it. The transcript still can.
#[test]
fn a_heredoc_write_still_names_its_author() {
    let command = "cat > /home/example/.claude/projects/-x/memory/feedback_new.md <<'MD'\nbody\nMD";
    assert!(writes(command, None, "/home/example", "feedback_new.md"));
}

#[test]
fn a_single_failure_names_the_memory_and_the_rule_in_the_subject() {
    let line = subject(&["feedback_new".to_string()], "missing-modified");
    assert!(line.starts_with(MARKER), "{line}");
    assert!(line.contains("feedback_new"), "{line}");
    assert!(line.contains("missing-modified"), "{line}");
}

/// ⚠ **One open task per agent, refreshed.** The nightly runs daily and an
/// unfixed error persists, so filing on every run turns one stale finding into
/// thirty tasks — a queue read as noise and then as nothing, which is the
/// failure this tool exists to fix, reproduced by the fix.
#[test]
fn an_existing_open_task_is_found_so_the_next_run_refreshes_it() {
    let listed = format!(
        r#"[{{"id":42,"status":"open","subject":"{MARKER} 3 of your memories fail the corpus lint"}}]"#
    );
    assert_eq!(open_task_in(&listed), Some(42));
}

/// ⚠ **Open only.** A closed task carrying the marker is a fixed error;
/// refreshing it would reopen a finished conversation instead of raising the
/// new one.
#[test]
fn a_closed_task_is_not_refreshed() {
    let listed = format!(
        r#"[{{"id":42,"status":"done","subject":"{MARKER} an error that was already fixed"}}]"#
    );
    assert_eq!(open_task_in(&listed), None);
}

#[test]
fn an_unrelated_open_task_is_not_mistaken_for_this_one() {
    let listed = r#"[{"id":7,"status":"open","subject":"something else entirely"}]"#;
    assert_eq!(open_task_in(listed), None);
}
