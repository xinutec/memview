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

use reader::shell_ops::Op;
use reader::{python, shell, shell_files};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: python-report <corpus.jsonl> [--show <n>] [--why <substring>]");
    };
    let show = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(25);
    // The check that settles doubt about a path: the program that named it,
    // printed whole. Every argument about invented paths in the shell reader was
    // ended by reading six lines of the equivalent.
    let why = args
        .iter()
        .position(|a| a == "--why")
        .and_then(|i| args.get(i + 1))
        .cloned();
    // The same question asked of the worklist: which program is it that calls
    // this thing I have never heard of. An unknown call is only worth teaching
    // once you have seen the code it came from.
    let sample = args
        .iter()
        .position(|a| a == "--sample")
        .and_then(|i| args.get(i + 1))
        .cloned();
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
        calls += 1;
        let Ok(parsed) = shell::parse(cmd) else {
            continue;
        };
        let found = shell_files::extract(&parsed, cwd, &home);
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
    let unknown: usize = tally.unknown.values().sum();
    println!("Bash calls            {calls}");
    println!("python programs       {}", tally.programs);
    println!(
        "  moved their own cwd {}  (relative paths dropped)",
        tally.chdir
    );
    println!("file operations       {recognised}");
    println!(
        "  named a file        {}  ({:.1}%)",
        recognised - unresolved,
        100.0 * (recognised - unresolved) as f64 / recognised.max(1) as f64
    );
    println!("  named none          {unresolved}  (computed, f-string, loop variable)");
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
