//! Extract every `Bash` call from the session transcripts, for the shell report.
//!
//!     cargo run --release --bin bash-corpus > /tmp/bash-corpus.jsonl
//!     cargo run --bin shell-report -- /tmp/bash-corpus.jsonl
//!
//! With `--said <path>`, a SECOND artefact is written in the same pass: what
//! the author said each command was for. See [`said`] for why it is a separate
//! file and not another field on the row.
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

/// Where the stated intents go, when `--said <path>` asks for them.
///
/// ⚠ **A SEPARATE FILE, and the reason is a boundary rather than a format.** A
/// `description` is a claim the author made about the command; the corpus row is
/// a record of what ran. Everything downstream of `bash-corpus.jsonl` is a
/// static reader, and prose is not evidence about execution — put the two in one
/// row and the only thing keeping a reader from consulting the prose is
/// discipline. Two files make it structural: a reader that wanted the intent
/// would have to go and open another artefact, which is a decision somebody
/// would have to write down.
///
/// ⚠ **The reason is NOT the retired union.** `docs/concept-model.md` first gave
/// one — that the row shape is load-bearing for memview#1130's collapse of a
/// duplicated era — and that argument died with `~/.claude/corpus/` on
/// 2026-08-29: there is no cumulative store any more, the corpus is re-mined
/// whole every night, and a shape change would simply change the whole file at
/// once. The boundary argument above is the one that survives.
///
/// **Joined on `(at, cmd)`**, which was measured before it was relied on: over
/// 450,866 described calls the pair resolves to 184,590 distinct keys and
/// **not one of them carries two different descriptions** (2026-09-03). The
/// same pass writes both files from the same deduplicated walk, so the join is
/// exact by construction rather than by luck.
///
/// A call with no description contributes no row at all. Absence is a fact the
/// report measures — 97.6% of calls carry one — and a row saying `null` would
/// be the same fact spelled expensively.
fn said_row(at: &Option<String>, command: &str, description: &str) -> serde_json::Value {
    let mut row = serde_json::json!({ "cmd": command, "said": description });
    if let Some(at) = at {
        row["at"] = serde_json::json!(at);
    }
    row
}

fn main() -> anyhow::Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut args = std::env::args().skip(1);
    let mut root = None;
    let mut said_to = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--said" => said_to = args.next(),
            _ => root = Some(arg),
        }
    }
    let root = root.unwrap_or_else(|| format!("{home}/.claude/projects"));
    // Opened before the walk, so a bad path fails in a second rather than four
    // minutes later with the corpus already on stdout.
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
        // What became of each call, gathered first because the answer is always
        // below the question.
        let mut outcomes: std::collections::HashMap<String, Verdict> =
            std::collections::HashMap::new();
        for line in text.lines() {
            if let Some((call, verdict)) = agents::tool_result(line.as_bytes()) {
                outcomes.insert(call, verdict);
            }
        }
        // ⚠ **What the shell said, which the verdict cannot carry.** `cd nope;
        // cat x` exits 0 — the verdict is `Ok` and the directory still never
        // moved, so a reader of this corpus applying the `cd` files every later
        // relative path under a directory the command never entered. The words
        // are the only evidence and they are in the transcript, not in the row.
        let refused = agents::refusals(text.as_bytes());
        // ⚠ **One row per CALL, not per line — a transcript holds the same line
        // more than once.** The CLI re-appends stretches of a conversation it
        // has already written, and this corpus emitted a row for each copy. The
        // whole corpus went **194,831 rows → 120,279** when this was added:
        // **38.3% of it was one call counted more than once** (memview #448).
        //
        // ⚠ **That skewed the corpus, not just inflated it.** The repeats are
        // concentrated in whichever sessions were rewritten most, so the shares
        // moved with them: `ssh` 3.49% → 5.33%, `nix-shell` 2.08% → 3.37%,
        // `sed` 5.85% → 4.30%. Every figure taken from this file before now —
        // counts, coverage, "the corpus does X N times" — is a figure about a
        // corpus that counted some sessions twice.
        //
        // Keyed on the call id, which is unique per call and shared by every
        // copy of the line carrying it. Per file, because copies only ever occur
        // within one transcript.
        //
        // ⚠ The FIRST copy is the one kept, deliberately. The copies are not
        // identical: measured, the later one carries a **shallower `cwd`** —
        // 114,464 of 114,464 differing pairs in one transcript, every one of
        // them re-stamped nearer the session root. Keeping the newest would
        // trade the directory a relative path needs for one that cannot resolve
        // it. See [[reference_transcript_cwd_is_both_before_and_after]].
        let mut emitted: std::collections::HashSet<String> = std::collections::HashSet::new();
        for line in text.lines() {
            // The same reader the miner uses, so the corpus a coverage figure is
            // measured against cannot drift from the text the miner parses.
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
                // No result at all is its own answer: the call was interrupted,
                // is still running, or the transcript ends mid-turn. An
                // interruption is not a result line but a separate message, so
                // this is the only trace it leaves.
                let ran = outcomes.get(&id).copied().unwrap_or(Verdict::Unknown);
                let mut row = serde_json::json!({ "cmd": command, "cwd": cwd, "ran": ran });
                // ⚠ **Carried so a command can be counted into a DAY.** Every
                // question about whether something happens MORE or LESS than it
                // used to needs this, and the union corpus cannot answer one:
                // it is distinct commands, so a shape run a hundred times is one
                // row with no time on it at all (memview #884's trap-incidence
                // arm).
                if let Some(at) = &at {
                    row["at"] = serde_json::json!(at);
                }
                // Written only when there is one — 247 calls in the whole corpus
                // carry a refusal, and an empty list on the other 880,000 rows is
                // megabytes saying nothing.
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
        // Both numbers, because the interesting figure is the SHARE: a said
        // count alone cannot say whether a fall is fewer calls or fewer
        // descriptions.
        let share = 100.0 * said_rows as f64 / calls.max(1) as f64;
        eprintln!("{said_rows} of them said what they were for ({share:.1}%) → {path}");
    }
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
