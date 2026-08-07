//! Extract every `Bash` call from the session transcripts, for the shell report.
//!
//!     cargo run --release --bin bash-corpus > /tmp/bash-corpus.jsonl
//!     cargo run --bin shell-report -- /tmp/bash-corpus.jsonl
//!
//! One JSON object per line: the command as written, the `cwd` it ran in, and
//! what became of it. The cwd is carried because a relative path names nothing
//! without it, and the transcripts record it on every line — it is the one piece
//! of context that cannot be recovered later.
//!
//! The outcome needs a second pass: a call's result is written further down the
//! file than the call, and a transcript is read once from the top.
//!
//! Lives here rather than in a scratchpad script because the coverage figure in
//! `shell.pest` is only checkable if the corpus behind it can be rebuilt.
//!
//! ⚠ **The odd one out of the report family.** Its four siblings moved to the
//! `reader` crate with the grammars they measure; this one stayed, because it is
//! the only one that reads *transcripts* rather than a corpus already extracted,
//! and transcript access is still the viewer's. It is what makes the corpus the
//! others consume, so it will follow them when that moves.

use std::io::Write;

use memview::agents;
use reader::doing::Verdict;

fn main() -> anyhow::Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{home}/.claude/projects"));

    let out = std::io::stdout();
    let mut out = std::io::BufWriter::new(out.lock());
    let mut calls = 0usize;
    let mut files = 0usize;

    for path in transcripts(std::path::Path::new(&root)) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        files += 1;
        // What became of each call, gathered first because the answer is always
        // below the question.
        let mut outcomes: std::collections::HashMap<String, Verdict> =
            std::collections::HashMap::new();
        for line in text.lines() {
            if let Some((call, verdict)) = agents::tool_result(line.as_bytes()) {
                outcomes.insert(call, verdict);
            }
        }
        for line in text.lines() {
            // The same reader the miner uses, so the corpus a coverage figure is
            // measured against cannot drift from the text the miner parses.
            let Some(agents::BashLine { cwd, calls: found }) =
                agents::bash_calls_with_ids(line.as_bytes())
            else {
                continue;
            };
            let cwd = cwd.unwrap_or_default();
            for agents::BashCall { id, command } in found {
                // No result at all is its own answer: the call was interrupted,
                // is still running, or the transcript ends mid-turn. An
                // interruption is not a result line but a separate message, so
                // this is the only trace it leaves.
                let ran = outcomes.get(&id).copied().unwrap_or(Verdict::Unknown);
                writeln!(
                    out,
                    "{}",
                    serde_json::json!({ "cmd": command, "cwd": cwd, "ran": ran })
                )?;
                calls += 1;
            }
        }
    }
    out.flush()?;
    eprintln!("{calls} Bash calls from {files} transcripts");
    Ok(())
}

/// Every `.jsonl` under the projects root, main-loop and delegated alike — a
/// subagent's shell is its dispatching session's work, the same rule the agent
/// miner follows.
fn transcripts(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            match entry.file_type() {
                // `file_type` does not follow symlinks, where `is_dir` would: a
                // link back to an ancestor would recurse until the stack gives out.
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(_) if path.extension().is_some_and(|e| e == "jsonl") => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}
