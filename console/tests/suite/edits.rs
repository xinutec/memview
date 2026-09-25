//! What a `Bash` call is predicted to change, and whether it did.

use std::path::{Path, PathBuf};

use console::edits::{Edits, Hunk, hunks};
use console::protocol::Ended;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("console-edits-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

fn at(dir: &Path) -> &str {
    dir.to_str().expect("utf-8")
}

fn numbered(lines: usize) -> String {
    (1..=lines).map(|n| format!("line {n}\n")).collect()
}

#[test]
fn a_change_keeps_three_lines_either_side() {
    let was = numbered(20);
    let now = was.replace("line 10\n", "line ten\n");
    assert_eq!(
        hunks("/f", &was, &now),
        vec![Hunk {
            path: "/f".to_string(),
            before: "line 7\nline 8\nline 9\nline 10\nline 11\nline 12\nline 13\n".to_string(),
            after: "line 7\nline 8\nline 9\nline ten\nline 11\nline 12\nline 13\n".to_string(),
        }]
    );
}

#[test]
fn changes_far_apart_are_separate_hunks() {
    let was = numbered(40);
    let now = was
        .replace("line 5\n", "five\n")
        .replace("line 35\n", "thirty-five\n");
    assert_eq!(hunks("/f", &was, &now).len(), 2);
}

#[test]
fn a_call_is_predicted_before_it_runs_against_what_the_file_holds() {
    let dir = scratch("predicted");
    std::fs::write(dir.join("a.txt"), "old\n").expect("seed");
    let edits = Edits::new(dir.join("store"));

    let edited = edits
        .before("s1", "c1", "cat > a.txt <<'EOF'\nnew\nEOF", at(&dir))
        .expect("predicted");

    assert_eq!(edited.hunks.len(), 1);
    assert_eq!(edited.hunks[0].before, "old\n");
    assert_eq!(edited.hunks[0].after, "new\n");
    assert_eq!(
        edits.of("s1").edited,
        vec![edited],
        "kept for the conversation"
    );
}

#[test]
fn an_append_is_predicted_from_the_file_it_extends() {
    let dir = scratch("append");
    std::fs::write(dir.join("a.txt"), "one\n").expect("seed");
    let edits = Edits::new(dir.join("store"));
    let edited = edits
        .before("s1", "c1", "echo two >> a.txt", at(&dir))
        .expect("predicted");
    assert_eq!(edited.hunks[0].after, "one\ntwo\n");
}

#[test]
fn a_call_that_did_what_was_predicted_agrees() {
    let dir = scratch("agrees");
    let edits = Edits::new(dir.join("store"));
    edits
        .before("s1", "c1", "echo x > a.txt", at(&dir))
        .expect("predicted");
    // What the command does; nothing here runs it.
    std::fs::write(dir.join("a.txt"), "x\n").expect("write");
    let (session, diverged) = edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    assert_eq!(session, "s1");
    assert!(diverged.paths.is_empty());
    assert!(edits.of("s1").diverged.is_empty());
}

#[test]
fn a_call_that_left_something_else_is_a_finding() {
    let dir = scratch("diverges");
    let edits = Edits::new(dir.join("store"));
    edits
        .before("s1", "c1", "echo x > a.txt", at(&dir))
        .expect("predicted");
    std::fs::write(dir.join("a.txt"), "not x\n").expect("write");
    let (_, diverged) = edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    assert_eq!(
        diverged.paths,
        vec![dir.join("a.txt").display().to_string()]
    );
    assert_eq!(edits.of("s1").diverged, vec![diverged]);
    let findings = std::fs::read_to_string(dir.join("store/findings.jsonl")).expect("kept");
    assert!(
        findings.contains("echo x > a.txt"),
        "the command travels with the finding"
    );
}

#[test]
fn a_command_the_reader_cannot_follow_predicts_nothing() {
    let dir = scratch("unfollowed");
    let edits = Edits::new(dir.join("store"));
    assert!(edits.before("s1", "c1", "make > a.txt", at(&dir)).is_none());
    assert!(edits.finished("c1", &serde_json::json!({})).is_none());
}

/// A call sent to the background has only started when its hook says it is done:
/// checking then compares the prediction with the files before it ran. The
/// check waits for the task to end, and a task that failed is not held to it.
#[test]
fn a_backgrounded_call_is_checked_when_its_task_ends() {
    let dir = scratch("background");
    std::fs::write(dir.join("a.txt"), "old\n").expect("seed");
    let edits = Edits::new(dir.join("store"));
    let command = "cat > a.txt <<'EOF'\nnew\nEOF";
    let backgrounded = serde_json::json!({ "stdout": "", "backgroundTaskId": "b1" });

    edits
        .before("s1", "c1", command, at(&dir))
        .expect("predicted");
    assert!(
        edits.finished("c1", &backgrounded).is_none(),
        "not checked yet"
    );
    std::fs::write(dir.join("a.txt"), "new\n").expect("the task ran");
    let (_, diverged) = edits
        .ended("c1", Some(&Ended::Completed))
        .expect("checked when it ended");
    assert!(diverged.paths.is_empty(), "{diverged:?}");

    // The control: the same call checked at once sees the file before it ran.
    std::fs::write(dir.join("a.txt"), "old\n").expect("reset");
    edits
        .before("s1", "c2", command, at(&dir))
        .expect("predicted");
    let (_, diverged) = edits
        .finished("c2", &serde_json::json!({ "stdout": "" }))
        .expect("checked at once");
    assert_eq!(diverged.paths.len(), 1);

    // A task that failed did not do what its text says, so nothing is checked.
    edits
        .before("s1", "c3", command, at(&dir))
        .expect("predicted");
    assert!(edits.finished("c3", &backgrounded).is_none());
    assert!(edits.ended("c3", Some(&Ended::Failed)).is_none());
    assert!(
        edits.ended("c3", Some(&Ended::Completed)).is_none(),
        "dropped"
    );
}
