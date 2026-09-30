//! What the fleet's sessions have actually been doing, in one vocabulary.
//!
//!     cargo run --release --bin activity-report -- <corpus.jsonl> [--show <n>] [--sample KIND]
//!
//! The vocabulary in `activity.rs` is grown from this the way the grammar was
//! grown from `shell-report`: what tops the unnamed tail is what to add next.

use std::collections::BTreeMap;

use clap::Parser;

use reader::activity::Activity;
use reader::shell_files;

/// What the fleet's sessions have actually been doing, in one vocabulary.
#[derive(Parser)]
struct Cli {
    /// The command corpus, from `bash-corpus`.
    corpus: String,
    /// How many rows of each table.
    #[arg(long, default_value_t = 20)]
    show: usize,
    /// Print sample commands of this kind.
    #[arg(long, value_name = "KIND")]
    sample: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let (path, show, sample) = (&cli.corpus, cli.show, cli.sample.clone());
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
        // The `cd` targets the shell refused, which only its own output knows —
        // see `agents::refusals`. Absent on all but a handful of rows.
        let refused: Vec<String> = row["refused"]
            .as_array()
            .map(|it| {
                it.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let Ok(parsed) = reader::project::read(cmd) else {
            continue;
        };
        let found = shell_files::extract_knowing(&parsed, cwd, &home, &refused);
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
