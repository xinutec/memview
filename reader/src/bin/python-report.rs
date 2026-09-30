//! What the Python reader can make of the history's Python, and what it cannot.
//!
//!     cargo run --release --bin python-report -- <corpus.jsonl> [--show <n>] [--why <substring>]
//!
//! The third of the family after `shell-report` (the grammar) and `shell-files`
//! (the shell's semantics). There is no parse-failure figure here on purpose:
//! `python.pest` accepts punctuation it has no reading for, so a program is
//! never rejected whole and the honest measure of coverage is **whether the call
//! was understood**. The worklist at the bottom is therefore the report — what
//! tops it is what to teach the reader next.

use std::collections::BTreeMap;

use clap::Parser;

use reader::shell_ops::Op;
use reader::{python, shell_files};

/// What the Python reader can make of the history's Python, and what it
/// cannot.
#[derive(Parser)]
struct Cli {
    /// The command corpus, from `bash-corpus`.
    corpus: String,
    /// How many rows of each table.
    #[arg(long, default_value_t = 25)]
    show: usize,
    /// Print whole the programs that named a path containing this: the check
    /// that settles doubt about a path.
    #[arg(long, value_name = "SUBSTRING")]
    why: Option<String>,
    /// Print the programs that call an unknown function containing this.
    #[arg(long, value_name = "SUBSTRING")]
    sample: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let (path, show) = (&cli.corpus, cli.show);
    let (why, sample) = (cli.why.clone(), cli.sample.clone());
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;
    let mut tally = python::Tally::default();
    let mut calls = 0usize;
    // The literals as the programs wrote them — unresolved, because whether a
    // path is real is judged on what was typed, not on where it landed.
    let mut paths: BTreeMap<String, (usize, usize)> = BTreeMap::new();
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
        calls += 1;
        let Ok(parsed) = reader::project::read(cmd) else {
            continue;
        };
        let found = shell_files::extract_knowing(&parsed, cwd, &home, &refused);
        for op in &found.ops {
            let Op::Python { source } = op else { continue };
            let program = python::read(source);
            if let Some(sample) = &sample
                && program.unknown.contains_key(sample.as_str())
                && witnessed < show
            {
                witnessed += 1;
                println!("--- calls {sample}:\n{}\n", source.trim_end());
            }
            for use_ in program.uses {
                let entry = paths.entry(use_.path.clone()).or_default();
                if use_.write {
                    entry.1 += 1;
                } else {
                    entry.0 += 1;
                }
                if let Some(why) = &why
                    && use_.path.contains(why.as_str())
                    && witnessed < show
                {
                    witnessed += 1;
                    let mark = if use_.write { "write" } else { "read " };
                    println!("{mark}  {}\n{}\n", use_.path, source.trim_end());
                }
            }
        }
        tally.merge(found.python);
    }

    let recognised: usize = tally.calls.values().sum();
    let unresolved: usize = tally.unresolved.values().sum();
    // Subtracted from the named, not added to them. These operations know
    // every path they could have used and not which one ran, so counting them
    // as named would move this rate without anything more being known — the
    // first version of this line read 87.2% while `file uses` had risen by
    // three. A bound is a better answer than a shrug; it is not a name.
    let bounded: usize = tally.bounded.values().sum();
    let unknown: usize = tally.unknown.values().sum();
    let named = recognised - unresolved - bounded;
    println!("Bash calls            {calls}");
    println!("python programs       {}", tally.programs);
    println!(
        "  moved their own cwd {}  (relative paths dropped)",
        tally.chdir
    );
    println!("file operations       {recognised}");
    println!(
        "  named a file        {named}  ({:.1}%)",
        100.0 * named as f64 / recognised.max(1) as f64
    );
    println!(
        "  one of a known set  {bounded}  ({} of them under one directory)",
        tally.located.values().sum::<usize>()
    );
    println!(
        "  named none          {unresolved}, and why — each row names the rule that would shrink it:"
    );
    for (why, n) in &tally.why {
        println!("    {:<34} {n}", why.name());
    }
    println!("file uses             {}", tally.uses);
    println!(
        "  kept as paths       {}  ({:.1}%)",
        tally.kept,
        100.0 * tally.kept as f64 / tally.uses.max(1) as f64
    );
    println!("distinct paths        {}", paths.len());
    println!("calls not understood  {unknown}");

    println!("\nfile operations, biggest first:");
    let mut ranked: Vec<_> = tally.calls.iter().collect();
    ranked.sort_by_key(|(name, n)| (std::cmp::Reverse(**n), (*name).clone()));
    for (name, n) in ranked.into_iter().take(show) {
        let missed = tally.unresolved.get(name).copied().unwrap_or(0);
        println!("  {n:>7}  {missed:>7} named nothing   {name}");
    }

    println!("\nbusiest paths (reads/writes), as the programs wrote them:");
    let mut ranked: Vec<_> = paths.into_iter().collect();
    ranked.sort_by_key(|(path, (r, w))| (std::cmp::Reverse(r + w), path.clone()));
    for (path, (r, w)) in ranked.into_iter().take(show) {
        println!("  {r:>6} {w:>6}  {path}");
    }

    println!("\ncalls this reader does not know — the worklist:");
    let mut ranked: Vec<_> = tally.unknown.into_iter().collect();
    ranked.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), name.clone()));
    for (name, n) in ranked.into_iter().take(show) {
        println!("  {n:>7}  {name}");
    }
    Ok(())
}
