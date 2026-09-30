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
            assumed: None,
            alternative: None,
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
fn a_finding_keeps_what_the_evaluator_was_given() {
    let dir = scratch("finding-inputs");
    std::fs::write(dir.join("a.txt"), "one\n").expect("seed");
    let edits = Edits::new(dir.join("store"));
    edits
        .before("s1", "c1", "echo two >> a.txt", at(&dir))
        .expect("predicted");
    std::fs::write(dir.join("a.txt"), "one\nthree\n").expect("write");
    edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    let findings = std::fs::read_to_string(dir.join("store/findings.jsonl")).expect("kept");
    let finding: serde_json::Value =
        serde_json::from_str(findings.lines().next().expect("one finding")).expect("json");
    assert_eq!(finding["cwd"], at(&dir));
    assert_eq!(
        finding["files"][dir.join("a.txt").display().to_string()],
        "one\n",
        "the file as it was before the call, so the prediction can be made again"
    );
}

/// Found live: `python3 - <<EOF … EOF; grep …` — the script raised before its
/// write and the shell's exit code was grep's, so the call counted as done.
/// The interpreter says so itself, and a call that did not do what its text
/// says is not held against the prediction.
#[test]
fn a_call_whose_interpreter_raised_is_not_judged() {
    let dir = scratch("raised");
    let edits = Edits::new(dir.join("store"));
    edits
        .before("s1", "c1", "echo x > a.txt; true", at(&dir))
        .expect("predicted");
    let response = serde_json::json!({
        "stdout": "",
        "stderr": "Traceback (most recent call last):\n  File \"<stdin>\", line 3, in <module>\nValueError: substring not found\n",
    });
    assert!(edits.finished("c1", &response).is_none());
    assert!(!dir.join("store/findings.jsonl").exists());
}

#[test]
fn old_and_undated_refusal_rows_are_pruned_when_one_is_kept() {
    let dir = scratch("prune");
    std::fs::create_dir_all(dir.join("store")).expect("store");
    std::fs::write(
        dir.join("store/refused.jsonl"),
        "{\"call\":\"old\",\"at\":\"2020-01-01T00:00:00Z\",\"refused\":[\"program x\"]}\n\
         {\"call\":\"undated\",\"refused\":[\"program x\"]}\n",
    )
    .expect("seed");
    let edits = Edits::new(dir.join("store"));
    assert!(edits.before("s1", "c1", "make > a.txt", at(&dir)).is_none());
    let rows = std::fs::read_to_string(dir.join("store/refused.jsonl")).expect("kept");
    assert_eq!(rows.lines().count(), 1, "{rows}");
    assert!(rows.contains("\"call\":\"c1\""));
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

/// A backgrounded call's files can be changed by something else before its task
/// ends — the session's next edit, which never passes through here. Found live: a
/// doc edit sent to the background with its commit, then edited again while the
/// commit's gate ran. So the files are looked at twice, when the call is
/// backgrounded and when its task ends, and either look holding the prediction
/// agrees. Neither holding it still diverges.
#[test]
fn a_backgrounded_call_agrees_if_either_look_holds_the_prediction() {
    let dir = scratch("overtaken");
    std::fs::write(dir.join("a.txt"), "old\n").expect("seed");
    let edits = Edits::new(dir.join("store"));
    let command = "cat > a.txt <<'EOF'\nnew\nEOF";
    let backgrounded = serde_json::json!({ "stdout": "", "backgroundTaskId": "b1" });

    edits
        .before("s1", "c1", command, at(&dir))
        .expect("predicted");
    std::fs::write(dir.join("a.txt"), "new\n").expect("the call wrote it at once");
    assert!(edits.finished("c1", &backgrounded).is_none());
    std::fs::write(dir.join("a.txt"), "edited again\n").expect("a later edit");
    let (_, diverged) = edits.ended("c1", Some(&Ended::Completed)).expect("checked");
    assert!(diverged.paths.is_empty(), "{diverged:?}");

    // The control: neither look holds it.
    std::fs::write(dir.join("a.txt"), "old\n").expect("reset");
    edits
        .before("s1", "c2", command, at(&dir))
        .expect("predicted");
    assert!(edits.finished("c2", &backgrounded).is_none());
    std::fs::write(dir.join("a.txt"), "something else\n").expect("wrong");
    let (_, diverged) = edits.ended("c2", Some(&Ended::Completed)).expect("checked");
    assert_eq!(diverged.paths.len(), 1);
}

/// What a live call's prediction refused is kept, by the census's names: live
/// calls have their files read, so these refusals — not history's `not read` —
/// are what to teach the evaluator next. A call that refused nothing adds nothing.
#[test]
fn a_live_calls_refusals_are_kept_by_name() {
    let dir = scratch("refused");
    let edits = Edits::new(dir.join("store"));
    assert!(edits.before("s1", "c1", "make > a.txt", at(&dir)).is_none());
    edits
        .before("s1", "c2", "echo x > b.txt", at(&dir))
        .expect("predicted");
    let kept = std::fs::read_to_string(dir.join("store").join("refused.jsonl")).expect("kept");
    let rows: Vec<serde_json::Value> = kept
        .lines()
        .map(|line| serde_json::from_str(line).expect("json"))
        .collect();
    assert_eq!(rows.len(), 1, "{kept}");
    assert_eq!(rows[0]["call"], "c1");
    assert_eq!(rows[0]["refused"], serde_json::json!(["program make"]));
}

/// A file the command removes is predicted gone, and the check holds it to that:
/// gone agrees, still there diverges.
#[test]
fn a_removed_file_is_checked_for_being_gone() {
    let dir = scratch("removed");
    let edits = Edits::new(dir.join("store"));
    std::fs::write(dir.join("a.txt"), "old\n").expect("seed");
    edits
        .before("s1", "c1", "rm a.txt", at(&dir))
        .expect("predicted");
    std::fs::remove_file(dir.join("a.txt")).expect("the call ran");
    let (_, diverged) = edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    assert!(diverged.paths.is_empty(), "{diverged:?}");

    std::fs::write(dir.join("a.txt"), "old\n").expect("seed");
    edits
        .before("s1", "c2", "rm a.txt", at(&dir))
        .expect("predicted");
    let (_, diverged) = edits
        .finished("c2", &serde_json::json!({}))
        .expect("checked");
    assert_eq!(diverged.paths.len(), 1, "a file still there is not removed");
}

/// The rows `outcomes.jsonl` holds, parsed.
fn outcomes(dir: &Path) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join("store/outcomes.jsonl"))
        .unwrap_or_default()
        .lines()
        .map(|line| serde_json::from_str(line).expect("json"))
        .collect()
}

#[test]
fn every_checked_file_leaves_an_outcome() {
    let dir = scratch("outcomes");
    let edits = Edits::new(dir.join("store"));
    edits
        .before("s1", "c1", "echo x > a.txt; echo y > b.txt", at(&dir))
        .expect("predicted");
    std::fs::write(dir.join("a.txt"), "x\n").expect("write");
    std::fs::write(dir.join("b.txt"), "not y\n").expect("write");
    edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    let rows = outcomes(&dir);
    let said: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| {
            (
                row["path"].as_str().expect("path"),
                row["outcome"].as_str().expect("outcome"),
            )
        })
        .collect();
    let (a, b) = (
        dir.join("a.txt").display().to_string(),
        dir.join("b.txt").display().to_string(),
    );
    assert_eq!(said, vec![(a.as_str(), "agreed"), (b.as_str(), "diverged")]);
    assert!(rows.iter().all(|row| row.get("assumed").is_none()));
}

#[test]
fn a_call_that_raised_leaves_its_files_unchecked() {
    let dir = scratch("unchecked");
    let edits = Edits::new(dir.join("store"));
    edits
        .before("s1", "c1", "echo x > a.txt", at(&dir))
        .expect("predicted");
    let response = serde_json::json!({"stderr": "Traceback (most recent call last):\n"});
    assert!(edits.finished("c1", &response).is_none());
    let rows = outcomes(&dir);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["outcome"], "unchecked");
    assert_eq!(rows[0]["why"], "raised");
}

/// Found live: a Python edit, then `scripts/dev bash -c 'cd lean && lake build'`.
/// The edit is drawn with the program it assumes, and checked like any other.
#[test]
fn a_file_an_unknown_program_may_have_touched_is_drawn_with_its_assumption() {
    let dir = scratch("conditional");
    let edits = Edits::new(dir.join("store"));
    let edited = edits
        .before("s1", "c1", "echo x > a.txt; ./build.sh", at(&dir))
        .expect("predicted");
    assert_eq!(edited.hunks.len(), 1);
    assert_eq!(edited.hunks[0].after, "x\n");
    let assumed = edited.hunks[0].assumed.as_ref().expect("conditional");
    assert_eq!(assumed.after, vec!["build.sh".to_string()]);
    assert!(assumed.before.is_empty());

    std::fs::write(dir.join("a.txt"), "x\n").expect("write");
    edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    let rows = outcomes(&dir);
    assert_eq!(rows[0]["outcome"], "agreed");
    assert_eq!(rows[0]["assumed"]["after"][0], "build.sh");
}

/// Either the evaluator or the program assumed harmless made the difference, so
/// the divergence is kept with its assumption and is not a finding.
#[test]
fn a_conditional_prediction_that_diverged_is_kept_apart_from_findings() {
    let dir = scratch("doubted");
    let edits = Edits::new(dir.join("store"));
    edits
        .before("s1", "c1", "echo x > a.txt; ./build.sh", at(&dir))
        .expect("predicted");
    std::fs::write(dir.join("a.txt"), "built\n").expect("write");
    let (_, diverged) = edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    assert_eq!(diverged.paths.len(), 1, "the drawn change did not happen");
    assert!(!dir.join("store/findings.jsonl").exists());
    let kept = std::fs::read_to_string(dir.join("store/conditional.jsonl")).expect("kept");
    let row: serde_json::Value = serde_json::from_str(kept.trim()).expect("json");
    assert_eq!(row["actual"], "built\n");
    assert_eq!(row["assumed"]["after"][0], "build.sh");
    assert_eq!(row["command"], "echo x > a.txt; ./build.sh");
}

/// A directory the evaluator lists is read from disk, used, and kept with the
/// row, so the prediction can be made again.
#[test]
fn a_listed_directory_is_used_and_kept_with_the_row() {
    let dir = scratch("listed");
    std::fs::write(dir.join("b.txt"), "B").expect("seed");
    std::fs::write(dir.join("a.txt"), "A").expect("seed");
    let edits = Edits::new(dir.join("store"));
    let edited = edits
        .before(
            "s1",
            "c1",
            "python3 - <<'PY'\nimport glob\nopen('o', 'w').write(','.join(sorted(glob.glob('*.txt'))))\nfor p in glob.glob('*.txt'):\n    open('log', 'a').write(p)\nPY",
            at(&dir),
        )
        .expect("predicted");
    assert_eq!(edited.hunks[0].after, "a.txt,b.txt");
    let kept =
        std::fs::read_to_string(dir.join("store/refused.jsonl")).expect("the loop is refused");
    let row: serde_json::Value = serde_json::from_str(kept.trim()).expect("json");
    let mut names: Vec<String> =
        serde_json::from_value(row["dirs"][at(&dir)].clone()).expect("the listing kept");
    names.sort();
    assert!(
        names.contains(&"a.txt".to_string()) && names.contains(&"b.txt".to_string()),
        "{names:?}"
    );
}

/// A file an undecided `if` leaves one of several texts is drawn once per text,
/// and agrees when it holds any of them.
#[test]
fn a_file_one_of_several_texts_is_drawn_per_text_and_agrees_on_any() {
    let dir = scratch("alternatives");
    std::fs::write(dir.join("f"), "x\n").expect("seed");
    std::fs::write(dir.join("o"), "old\n").expect("seed");
    let edits = Edits::new(dir.join("store"));
    // A permission sight does not decide.
    let command = "if [ -r f ]; then echo a > o; else echo b > o; fi";
    let edited = edits
        .before("s1", "c1", command, at(&dir))
        .expect("predicted");
    let drawn: Vec<(usize, usize, &str)> = edited
        .hunks
        .iter()
        .map(|hunk| {
            let alternative = hunk.alternative.expect("one of several");
            (alternative.at, alternative.of, hunk.after.as_str())
        })
        .collect();
    assert_eq!(drawn.len(), 2, "{drawn:?}");
    assert!(drawn.iter().all(|(_, of, _)| *of == 2));

    std::fs::write(dir.join("o"), "b\n").expect("write");
    let (_, diverged) = edits
        .finished("c1", &serde_json::json!({}))
        .expect("checked");
    assert!(diverged.paths.is_empty());
    let rows = outcomes(&dir);
    assert_eq!(rows[0]["outcome"], "agreed");
    assert_eq!(rows[0]["of"], 2);

    edits
        .before("s1", "c2", command, at(&dir))
        .expect("predicted");
    std::fs::write(dir.join("o"), "c\n").expect("write");
    let (_, diverged) = edits
        .finished("c2", &serde_json::json!({}))
        .expect("checked");
    assert_eq!(diverged.paths.len(), 1);
    let findings = std::fs::read_to_string(dir.join("store/findings.jsonl")).expect("kept");
    let finding: serde_json::Value = serde_json::from_str(findings.trim()).expect("json");
    assert_eq!(finding["actual"], "c\n");
    assert_eq!(finding["alternatives"].as_array().map(Vec::len), Some(2));
}
