//! Python's floats, held to what CPython printed: `python-floats.jsonl`, written
//! by `python-floats.py` beside it.

use reader::predict::{Files, Written, predict};

/// Each value read back from its own `repr`, then printed three ways.
#[test]
fn a_float_prints_and_rounds_as_cpython_does() {
    let mut wrong = Vec::new();
    for line in include_str!("python-floats.jsonl").lines() {
        let row: serde_json::Value = serde_json::from_str(line).expect("a row");
        let repr = row["repr"].as_str().expect("repr");
        let round2 = row["round2"].as_str().expect("round2");
        let (fixed, expected) = match row["fixed3"].as_str() {
            Some(fixed3) => (
                r#" + "\n" + f"{x:.3f}""#,
                format!("{repr}\n{round2}\n{fixed3}"),
            ),
            None => ("", format!("{repr}\n{round2}")),
        };
        let script = format!(
            "python3 - <<'EOF'\nx = {repr}\nopen('o.txt', 'w').write(str(x) + \"\\n\" + str(round(x, 2)){fixed})\nEOF"
        );
        let parsed = reader::syntax::parse(&script).expect("parses");
        let found = predict(&parsed, "/repo", "/home/me", &Files::new());
        let want = vec![Written {
            path: "/repo/o.txt".to_string(),
            text: Some(expected.clone()),
        }];
        if found.written != want {
            wrong.push(format!("{repr}: {:?} != {expected:?}", found.written));
        }
    }
    assert!(
        wrong.is_empty(),
        "{} wrong, first: {:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(5)]
    );
}
