//! What the fleet's sessions have actually been doing, in one vocabulary.
//!
//!     cargo run --release --bin activity-report -- <corpus.jsonl> [--show <n>] [--sample KIND]
//!
//! The vocabulary in `activity.rs` is grown from this the way the grammar was
//! grown from `shell-report`: what tops the unnamed tail is what to add next.

use std::collections::BTreeMap;

use reader::activity::Activity;
use reader::{shell, shell_files};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: activity-report <corpus.jsonl> [--show <n>] [--sample KIND]");
    };
    let show = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(20);
    let sample = args
        .iter()
        .position(|a| a == "--sample")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut unnamed: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    let mut witnessed = 0usize;

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        let Ok(parsed) = shell::parse(cmd) else {
            continue;
        };
        let found = shell_files::extract(&parsed, cwd, &home);
        // Taken from the extractor rather than re-derived here: it is the only
        // place an operation and the command it came from are paired, nested
        // shells included.
        for activity in &found.activities {
            total += 1;
            *kinds.entry(activity.label().to_string()).or_insert(0) += 1;
            if let Activity::Other { name } = activity {
                *unnamed.entry(name.clone()).or_insert(0) += 1;
            }
            if let Some(want) = &sample
                && activity.label() == want
                && witnessed < show
            {
                witnessed += 1;
                println!(
                    "  {}",
                    cmd.replace('\n', "⏎").chars().take(110).collect::<String>()
                );
            }
        }
    }

    let named: usize = kinds
        .iter()
        .filter(|(k, _)| !unnamed.contains_key(*k))
        .map(|(_, n)| n)
        .sum();
    println!("\ncommands            {total}");
    println!(
        "  named             {named}  ({:.1}%)",
        100.0 * named as f64 / total.max(1) as f64
    );
    println!("  unnamed           {}", total - named);

    println!("\nwhat the sessions do:");
    let mut ranked: Vec<_> = kinds
        .iter()
        .filter(|(k, _)| !unnamed.contains_key(*k))
        .collect();
    ranked.sort_by_key(|(k, n)| (std::cmp::Reverse(**n), (*k).clone()));
    for (kind, n) in ranked {
        println!("  {n:>8}  {kind}");
    }

    println!("\nnot in the vocabulary — the worklist:");
    let mut ranked: Vec<_> = unnamed.into_iter().collect();
    ranked.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), name.clone()));
    for (name, n) in ranked.into_iter().take(show) {
        println!("  {n:>8}  {name}");
    }
    Ok(())
}
