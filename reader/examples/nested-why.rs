//! Which nested scripts will not read, at every depth, and what they look like.
//!
//!     cargo run --release -p reader --example nested-why -- <corpus.jsonl> [--reason NAME]
//!
//! `shell-files` ranks the refusals by reason; this prints the payloads behind
//! one of them, because a reason name says which construct and not which shape.
//!
//! ⚠ **It walks to the bottom, not to depth 1.** The first version re-parsed
//! only the payloads it found in a top-level command, so a refusal inside
//! `ssh host 'bash -c "…"'` was counted by `shell-files` and invisible here —
//! which is how `Loop`, `FunctionDefinition`, `Arithmetic` and `Tilde` came to
//! show occurrences against 0 distinct scripts (memview #1028).
//!
//! **Termination is structural, not budgeted.** A payload is a proper part of
//! the command that carries it, so every step strictly shortens the text and
//! the walk cannot run forever. `depth` is carried to say WHERE a refusal was
//! found; nothing is cut off when it grows. See `feedback_no_fuel_limits`.
use std::collections::{BTreeMap, BTreeSet};

use reader::shell_ops::Op;

/// What one walk of the corpus found, keyed so a reason can be read per depth.
#[derive(Default)]
struct Found {
    /// Occurrences of each (reason, depth) — one per nested command met.
    occurrences: BTreeMap<(String, usize), usize>,
    /// Distinct refused payloads per reason, so a repeat does not inflate it.
    distinct: BTreeMap<String, BTreeSet<String>>,
    /// Deepest payload reached, refused or not.
    deepest: usize,
    /// Distinct payloads of the reason asked for, in the order first met.
    wanted: Vec<String>,
    seen: BTreeSet<String>,
}

/// Read `script`, and walk into every nested or remote payload it carries.
///
/// A payload that itself reads is descended into; one that refuses is recorded
/// and not descended into, because the tree past a refusal is not ours to read.
fn walk(script: &str, depth: usize, want: &str, found: &mut Found) {
    found.deepest = found.deepest.max(depth);
    let Ok(ran) = reader::project::read(script) else {
        return;
    };
    for simple in &ran.commands {
        let op = reader::shell_ops::classify(&simple.argv, &simple.heredocs, None, "/home/example");
        let payload = match &op {
            Op::Nested { script } | Op::Remote { script, .. } => script.clone(),
            _ => continue,
        };
        match reader::project::read(&payload) {
            Ok(_) => walk(&payload, depth + 1, want, found),
            Err(refusal) => {
                let reason = format!("{:?}", refusal.reason);
                *found
                    .occurrences
                    .entry((reason.clone(), depth + 1))
                    .or_default() += 1;
                found
                    .distinct
                    .entry(reason.clone())
                    .or_default()
                    .insert(payload.clone());
                if reason == want && found.seen.insert(payload.clone()) {
                    found.wanted.push(payload);
                }
            }
        }
    }
}

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

    let mut found = Found::default();
    for line in std::fs::read_to_string(path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        walk(cmd, 0, &want, &mut found);
    }

    // Per reason, then per depth, so "which depth is this refusal at" is read
    // off the table rather than inferred from a total.
    eprintln!("deepest payload reached: depth {}", found.deepest);
    eprintln!("{:<22} {:>6} {:>9}  by depth", "reason", "occ", "distinct");
    let reasons: BTreeSet<&String> = found.occurrences.keys().map(|(r, _)| r).collect();
    for reason in reasons {
        let by_depth: Vec<String> = found
            .occurrences
            .iter()
            .filter(|((r, _), _)| r == reason)
            .map(|((_, d), n)| format!("{d}:{n}"))
            .collect();
        let occ: usize = found
            .occurrences
            .iter()
            .filter(|((r, _), _)| r == reason)
            .map(|(_, n)| n)
            .sum();
        let distinct = found.distinct.get(reason).map_or(0, BTreeSet::len);
        eprintln!(
            "{reason:<22} {occ:>6} {distinct:>9}  {}",
            by_depth.join(" ")
        );
    }

    // ⚠ **"We refuse it" and "it is not shell" are different facts**, and only
    // bash settles the second. This crate must not spawn a process — see
    // `reader/src/lib.rs` — so the payloads go out NUL-separated and whoever
    // wants the verdict asks `bash -n` themselves.
    eprintln!(
        "\n{} distinct nested scripts refused with {want}",
        found.wanted.len()
    );
    for script in &found.wanted {
        print!("{script}\0");
    }
    Ok(())
}
