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
    /// Occurrences of each (reason, carrier) — the command that HANDED the
    /// payload over. A reason names the construct that would not parse; this
    /// names who produced the text, which is the half that says whether the
    /// fix is a grammar rule or a wrong model of the carrier.
    carriers: BTreeMap<(String, String), usize>,
    /// Deepest payload reached, refused or not.
    deepest: usize,
    /// Distinct payloads of the reason asked for, in the order first met.
    wanted: Vec<String>,
    seen: BTreeSet<String>,
}

/// Who handed the payload over: the carrier command, named the way the corpus
/// spells it. `kubectl exec` and `docker exec` are told apart from their other
/// subcommands because only those two carry a payload, and whether that payload
/// went through a SHELL is the whole question for them.
fn carrier_of(argv: &[String]) -> String {
    let name = argv
        .first()
        .map(|w| w.rsplit('/').next().unwrap_or(w).to_string())
        .unwrap_or_default();
    match name.as_str() {
        "kubectl" | "docker" => {
            let payload_is_a_shell = argv
                .iter()
                .skip_while(|w| w.as_str() != "--")
                .nth(1)
                .map(|w| w.rsplit('/').next().unwrap_or(w))
                .is_some_and(|w| matches!(w, "sh" | "bash" | "zsh" | "dash" | "ksh"));
            format!(
                "{name} exec -- {}",
                if payload_is_a_shell {
                    "sh -c"
                } else {
                    "a program"
                }
            )
        }
        _ => name,
    }
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
        let (payload, carrier) = match &op {
            Op::Nested { script } => (script.clone(), carrier_of(&simple.argv)),
            Op::Remote { script, .. } => (script.clone(), carrier_of(&simple.argv)),
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
                *found.carriers.entry((reason.clone(), carrier)).or_default() += 1;
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

    // Who handed over the text that would not read. A reason ranks the
    // construct; this ranks the CARRIER, and the two disagree on purpose.
    eprintln!("\nby carrier — who handed the refused payload over:");
    let mut by_carrier: Vec<(&(String, String), &usize)> = found.carriers.iter().collect();
    by_carrier.sort_by(|a, b| b.1.cmp(a.1));
    for ((reason, carrier), n) in by_carrier.iter().take(20) {
        eprintln!("{n:>6}  {reason:<22} {carrier}");
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
