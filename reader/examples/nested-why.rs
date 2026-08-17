//! Which nested scripts will not read, and what they look like.
//!
//!     cargo run --release -p reader --example nested-why -- <corpus.jsonl> [--reason NAME]
//!
//! `shell-files` ranks the refusals by reason; this prints the payloads behind
//! one of them, because a reason name says which construct and not which shape.
use reader::shell_ops::Op;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: nested-why <corpus.jsonl> [--reason NAME]");
    let want = args
        .iter()
        .position(|a| a == "--reason")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "Grouping".to_string());
    let mut shown = 0;
    let mut payloads: Vec<String> = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let Ok(ran) = reader::project::read(cmd) else {
            continue;
        };
        for simple in &ran.commands {
            let op =
                reader::shell_ops::classify(&simple.argv, &simple.heredocs, None, "/home/example");
            let script = match &op {
                Op::Nested { script } | Op::Remote { script, .. } => script.clone(),
                _ => continue,
            };
            if let Err(refusal) = reader::project::read(&script)
                && format!("{:?}", refusal.reason) == want
                && seen.insert(script.clone())
            {
                shown += 1;
                payloads.push(script.clone());
            }
        }
    }
    // ⚠ **"We refuse it" and "it is not shell" are different facts**, and only
    // bash settles the second. This crate must not spawn a process — see
    // `reader/src/lib.rs` — so the payloads go out NUL-separated and whoever
    // wants the verdict asks `bash -n` themselves.
    eprintln!("{shown} distinct nested scripts refused with {want}");
    for script in &payloads {
        print!("{script}\0");
    }
    Ok(())
}
