//! Extract every `Bash` call from the session transcripts, for the shell report.
//!
//!     cargo run --release --bin bash-corpus > /tmp/bash-corpus.jsonl
//!     cargo run --bin shell-report -- /tmp/bash-corpus.jsonl
//!
//! One JSON object per line: the command as written, and the `cwd` it ran in.
//! The cwd is carried because a relative path names nothing without it, and the
//! transcripts record it on every line — it is the one piece of context that
//! cannot be recovered later.
//!
//! Lives here rather than in a scratchpad script because the coverage figure in
//! `shell.pest` is only checkable if the corpus behind it can be rebuilt.

use std::io::Write;

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
        for line in text.lines() {
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let cwd = row["cwd"].as_str().unwrap_or_default();
            let Some(content) = row["message"]["content"].as_array() else {
                continue;
            };
            for item in content {
                if item["type"] != "tool_use" || item["name"] != "Bash" {
                    continue;
                }
                let Some(cmd) = item["input"]["command"].as_str() else {
                    continue;
                };
                writeln!(out, "{}", serde_json::json!({ "cmd": cmd, "cwd": cwd }))?;
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
