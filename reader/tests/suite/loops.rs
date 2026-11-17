//! A `while`, the ordering and identity comparisons, and a list's `pop`, `index`
//! and `count`, held to what CPython left in the file: `python-loops.jsonl`,
//! written by `python-loops.py` beside it.

use reader::predict::{Files, predict};

/// Every program's file is predicted as CPython left it or refused, and the
/// forms marked `must` are predicted. A program that raised is no claim: the
/// check skips a call that failed.
#[test]
fn a_loop_and_what_decides_it_run_as_cpython_runs_them() {
    let (mut wrong, mut refused) = (Vec::new(), Vec::new());
    let mut predicted = 0;
    for line in include_str!("python-loops.jsonl").lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("a row");
        let program = row["program"].as_str().expect("program");
        let Some(left) = row["left"].as_str() else {
            continue;
        };
        let files: Files = [(
            "/repo/t.txt".to_string(),
            Some(row["text"].as_str().expect("text").to_string()),
        )]
        .into_iter()
        .collect();
        let script = format!("python3 - <<'EOF'\n{program}\nEOF");
        let parsed = reader::syntax::parse(&script).expect("parses");
        let found = predict(&parsed, "/repo", "/home/me", &files);
        let said = found
            .written
            .iter()
            .find(|written| written.path == "/repo/o.txt")
            .and_then(|written| written.text.clone());
        match said {
            Some(said) if said == left => predicted += 1,
            Some(said) => wrong.push(format!("{program}\n  said {said:?}\n  left {left:?}")),
            None if row["must"] == true => refused.push(program.to_string()),
            None => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert!(refused.is_empty(), "refused:\n{}", refused.join("\n---\n"));
    assert!(predicted >= 25, "only {predicted} predicted");
}
