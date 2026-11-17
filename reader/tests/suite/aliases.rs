//! One object under two names, held to what CPython left in the file:
//! `python-aliases.jsonl`, written by `python-aliases.py` beside it.

use reader::predict::{Files, predict};

/// Every program's file is predicted as CPython left it or refused, and the
/// forms marked `must` are predicted.
#[test]
fn a_store_through_a_second_name_lands_where_cpython_puts_it() {
    let (mut wrong, mut refused) = (Vec::new(), Vec::new());
    let mut predicted = 0;
    for line in include_str!("python-aliases.jsonl").lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("a row");
        let program = row["program"].as_str().expect("program");
        let files: Files = [(
            "/repo/j.json".to_string(),
            Some(row["start"].as_str().expect("start").to_string()),
        )]
        .into_iter()
        .collect();
        let script = format!("python3 - <<'EOF'\n{program}EOF");
        let parsed = reader::syntax::parse(&script).expect("parses");
        let found = predict(&parsed, "/repo", "/home/me", &files);
        let said = found
            .written
            .iter()
            .find(|written| written.path == "/repo/j.json")
            .and_then(|written| written.text.clone());
        match (said, row["left"].as_str()) {
            (Some(said), Some(left)) if said == left => predicted += 1,
            (Some(said), left) => wrong.push(format!("{program}\n  said {said}\n  left {left:?}")),
            (None, _) if row["must"] == true => refused.push(program.to_string()),
            (None, _) => {}
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    assert!(refused.is_empty(), "refused:\n{}", refused.join("\n"));
    assert!(predicted >= 15, "only {predicted} predicted");
}
