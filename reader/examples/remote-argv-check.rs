//! What changed when a container payload stopped being joined into a script.
//!
//!     cargo run --release -p reader --example remote-argv-check -- <corpus.jsonl>
//!
//! `kubectl exec -- <program>` used to have its words joined with spaces and the
//! result parsed as shell; it is now classified as the argv it is (memview#1028,
//! [`Op::RemoteRun`]). That removed 740 refusals — and a report showing 20 fewer
//! reads at the same time is a claim that needs its own evidence, because "fewer
//! refusals" and "fewer findings" can be the same change seen twice.
//!
//! So this computes BOTH readings from the same payload and prints where they
//! disagree. The old one is reconstructed here rather than kept in the library:
//! a model nothing believes should not have a maintained implementation.
use std::collections::BTreeMap;

use reader::shell_ops::{Op, classify};

/// The paths one op names, flattened — direction dropped, since the question is
/// which SUBJECTS a reading finds, not what it does to them.
fn subjects(op: &Op) -> Vec<String> {
    match op {
        Op::Read { paths } | Op::Write { paths } | Op::Remove { paths, .. } => paths.clone(),
        Op::Search { paths, .. } => paths.clone(),
        Op::Transform { paths, .. } => paths.clone(),
        Op::Copy { from, to } | Op::Move { from, to } => {
            from.iter().chain(std::iter::once(to)).cloned().collect()
        }
        Op::Run { script } => vec![script.clone()],
        _ => Vec::new(),
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .expect("usage: remote-argv-check <corpus.jsonl>");
    let home = "/home/example";
    let mut payloads = 0usize;
    let mut agree = 0usize;
    // Payload → (what the old reading found, what the new one finds).
    let mut lost: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();
    let mut gained: BTreeMap<String, (Vec<String>, Vec<String>)> = BTreeMap::new();

    for line in std::fs::read_to_string(&path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        walk(cmd, home, &mut payloads, &mut agree, &mut lost, &mut gained);
    }

    println!("container payloads that are not a shell: {payloads}");
    println!("  both readings name the same subjects:  {agree}");
    println!("  the OLD reading named something more:  {}", lost.len());
    println!("  the NEW reading names something more:  {}", gained.len());
    println!("\nsubjects only the old (join-and-parse) reading found:");
    for (payload, (only_before, _)) in lost.iter().take(25) {
        println!("  {only_before:?}");
        println!(
            "      {}",
            payload
                .replace('\n', "⏎")
                .chars()
                .take(120)
                .collect::<String>()
        );
    }
    println!("\nsubjects only the new (classify-the-argv) reading finds:");
    for (payload, (_, only_now)) in gained.iter().take(15) {
        println!("  {only_now:?}");
        println!(
            "      {}",
            payload
                .replace('\n', "⏎")
                .chars()
                .take(120)
                .collect::<String>()
        );
    }
    Ok(())
}

/// Every container payload in `script`, at any depth.
///
/// ⚠ **Depth is the whole point.** Only 120 of these are top-level; the rest sit
/// inside an `ssh host '…'`, which is exactly how the refusal they caused came to
/// be counted by `shell-files` and invisible to the first version of this probe.
fn walk(
    script: &str,
    home: &str,
    payloads: &mut usize,
    agree: &mut usize,
    lost: &mut BTreeMap<String, (Vec<String>, Vec<String>)>,
    gained: &mut BTreeMap<String, (Vec<String>, Vec<String>)>,
) {
    let Ok(ran) = reader::project::read(script) else {
        return;
    };
    for simple in &ran.commands {
        match classify(&simple.argv, &simple.heredocs, None, home) {
            Op::Nested { script } | Op::Remote { script, .. } => {
                walk(&script, home, payloads, agree, lost, gained);
            }
            Op::RemoteRun { argv, .. } => {
                *payloads += 1;
                // The new reading: one argv, classified.
                let mut now = subjects(&classify(&argv, &[], None, home));
                // The old reading: joined, parsed as shell, each command classified.
                let mut before = match reader::project::read(&argv.join(" ")) {
                    Ok(inner) => inner
                        .commands
                        .iter()
                        .flat_map(|c| subjects(&classify(&c.argv, &c.heredocs, None, home)))
                        .collect(),
                    Err(_) => Vec::new(),
                };
                before.sort();
                now.sort();
                if before == now {
                    *agree += 1;
                    continue;
                }
                let key = argv.join(" ");
                let only_before: Vec<String> = before
                    .iter()
                    .filter(|p| !now.contains(p))
                    .cloned()
                    .collect();
                let only_now: Vec<String> = now
                    .iter()
                    .filter(|p| !before.contains(p))
                    .cloned()
                    .collect();
                if !only_before.is_empty() {
                    lost.insert(key.clone(), (only_before, only_now.clone()));
                }
                if !only_now.is_empty() {
                    gained.insert(key, (Vec::new(), only_now));
                }
            }
            _ => {}
        }
    }
}
