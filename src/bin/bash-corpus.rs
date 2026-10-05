//! Extract every `Bash` call from the session transcripts, for the shell report.
//!
//!     cargo run --release --bin bash-corpus > /tmp/bash-corpus.jsonl
//!     cargo run --bin shell-report -- /tmp/bash-corpus.jsonl
//!
//! One JSON object per line: the command, the `cwd` it ran in — the one piece
//! of context that cannot be recovered later — what became of it, which needs a
//! second pass since a result is written below its call, and which instruction
//! it was part of: `session` is the transcript, `turn` the count of distinct
//! prompts above the call, so the calls one instruction produced share a pair.
//! That is the episode as `doing.rs` observes it — bracketed by the user's
//! turns, never inferred from a gap — and `episode-report` groups on it. A
//! subagent's transcript is a session of its own, as it is a file of its own.
//! With `--said <path>` a second artefact holds what the author said each
//! command was for; see [`said_row`]. The one report tool still in the viewer,
//! because it reads transcripts.

use std::io::Write;

use clap::Parser;

use memview::agents;
use reader::doing::Verdict;

/// Where the stated intents go, when `--said <path>` asks for them. A SEPARATE
/// FILE, for a boundary: a `description` is a claim the author made, the row is a
/// record of what ran, and everything downstream is a static reader — two files
/// make consulting the prose a decision somebody has to write down. Joined on
/// `(at, cmd)`, measured: 184,590 distinct keys and not one carries two
/// descriptions. A call with no description contributes no row; 97.6% carry one.
fn said_row(at: &Option<String>, command: &str, description: &str) -> serde_json::Value {
    let mut row = serde_json::json!({ "cmd": command, "said": description });
    if let Some(at) = at {
        row["at"] = serde_json::json!(at);
    }
    row
}

/// Extract every `Bash` call from the session transcripts, for the shell report.
#[derive(Parser)]
struct Cli {
    /// The projects root [default: ~/.claude/projects].
    root: Option<String>,
    /// Also write what the author said each command was for, to this file.
    #[arg(long, value_name = "FILE")]
    said: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let home = std::env::var("HOME").unwrap_or_default();
    let root = cli
        .root
        .unwrap_or_else(|| format!("{home}/.claude/projects"));
    let said_to = cli.said;
    // Opened before the walk, so a bad path fails in a second.
    let mut said = match &said_to {
        Some(path) => Some(std::io::BufWriter::new(std::fs::File::create(path)?)),
        None => None,
    };
    let mut said_rows = 0usize;

    let out = std::io::stdout();
    let mut out = std::io::BufWriter::new(out.lock());
    let mut calls = 0usize;
    let mut files = 0usize;

    for path in transcripts(std::path::Path::new(&root)) {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        files += 1;
        // What became of each call, gathered first: the answer is always below the question.
        let mut outcomes: std::collections::HashMap<String, Verdict> =
            std::collections::HashMap::new();
        for line in text.lines() {
            if let Some((call, verdict)) = agents::tool_result(line.as_bytes()) {
                outcomes.insert(call, verdict);
            }
        }
        // What the shell said, which the verdict cannot carry: `cd nope; cat x` exits 0.
        let refused = agents::refusals(text.as_bytes());
        // One row per CALL, not per line: the CLI re-appends stretches it has already
        // written, and this corpus was 38.3% repeats (194,831 rows → 120,279, memview
        // #448), skewing shares with them. Keyed on the call id, per file. The FIRST
        // copy is kept: the later one carries a shallower `cwd`, re-stamped nearer the
        // session root (`reference_transcript_cwd_is_both_before_and_after`).
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        let session = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        // Distinct prompts seen so far: the instruction a call belongs to. By
        // id, so a re-appended stretch of transcript does not open a new one —
        // a split is only noise to a consumer, a merge is unrecoverable, and
        // a prompt line with no id errs toward the split.
        let mut prompts: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut turn = 0u32;
        for line in text.lines() {
            if agents::is_prompt(line.as_bytes()) {
                let new = match agents::prompt_id(line.as_bytes()) {
                    Some(id) => prompts.insert(id),
                    None => true,
                };
                if new {
                    turn += 1;
                }
                continue;
            }
            // The same reader the miner uses, so a coverage figure cannot drift from it.
            let Some(agents::BashLine {
                cwd,
                at,
                calls: found,
            }) = agents::bash_calls_with_ids(line.as_bytes())
            else {
                continue;
            };
            let cwd = cwd.unwrap_or_default();
            for agents::BashCall {
                id,
                command,
                description,
            } in found
            {
                if !emitted.insert(id.clone()) {
                    continue;
                }
                // No result at all is its own answer: interrupted, still running, or the
                // transcript ends mid-turn.
                let ran = outcomes.get(&id).copied().unwrap_or(Verdict::Unknown);
                let mut row = serde_json::json!({
                    "cmd": command, "cwd": cwd, "ran": ran, "session": session, "turn": turn,
                });
                // Carried so a command can be counted into a DAY; a distinct-commands corpus
                // cannot say whether something happens more or less than it used to.
                if let Some(at) = &at {
                    row["at"] = serde_json::json!(at);
                }
                // Written only when there is one: 247 calls carry a refusal.
                if let Some(targets) = refused.get(&id) {
                    row["refused"] = serde_json::json!(targets);
                }
                writeln!(out, "{row}")?;
                calls += 1;
                if let (Some(said), Some(description)) = (said.as_mut(), &description) {
                    writeln!(said, "{}", said_row(&at, &command, description))?;
                    said_rows += 1;
                }
            }
        }
    }
    out.flush()?;
    if let Some(said) = said.as_mut() {
        said.flush()?;
    }
    eprintln!("{calls} Bash calls from {files} transcripts");
    if let Some(path) = &said_to {
        // Both numbers, because the interesting figure is the SHARE.
        let share = 100.0 * said_rows as f64 / calls.max(1) as f64;
        eprintln!("{said_rows} of them said what they were for ({share:.1}%) → {path}");
    }
    Ok(())
}

/// Every `.jsonl` under the projects root, delegated alike — a subagent's shell
/// is its dispatching session's work.
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
                // `file_type` does not follow symlinks, where `is_dir` would recurse.
                Ok(kind) if kind.is_dir() => stack.push(path),
                Ok(_) if path.extension().is_some_and(|e| e == "jsonl") => out.push(path),
                _ => {}
            }
        }
    }
    out.sort();
    out
}
