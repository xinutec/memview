//! Whether the conversations on this machine are intact.
//!
//! The readers in this workspace are lenient by design — they skip what they
//! cannot parse and carry on — so a transcript that had lost a message rendered
//! exactly like one that had not, and their silence proved nothing. This is the
//! counterpart that exists to say no.
//!
//! ⚠ **Leniency was never the standard, it was a habit.** `couse` walks
//! `parentUuid` chains to inherit `promptId` down a tree, so a broken link
//! silently truncates a walk and changes a published number with nothing
//! reported. The one real reason to tolerate anything is the append race, and
//! it is handled precisely rather than generally: see `Tail`.
//!
//!     transcript-lint                 # ~/.claude/projects
//!     transcript-lint DIR_OR_FILE
//!     transcript-lint --quiet DIR     # totals only
//!
//! Exit status is 0 only if every file is intact. A file still being written
//! may report `incomplete-tail`, which is not damage and does not fail the run.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use reader::transcript::{self, Tail};

fn main() -> Result<()> {
    let mut quiet = false;
    let mut examples = 3usize;
    let mut target: Option<PathBuf> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--quiet" => quiet = true,
            "--examples" => {
                examples = args
                    .next()
                    .and_then(|n| n.parse().ok())
                    .context("--examples wants a number")?;
            }
            other => target = Some(PathBuf::from(other)),
        }
    }

    let root = target.unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_default();
        PathBuf::from(home).join(".claude/projects")
    });

    let files = if root.is_dir() {
        let mut found = Vec::new();
        walk(&root, &mut found)?;
        found.sort();
        found
    } else {
        vec![root.clone()]
    };
    if files.is_empty() {
        anyhow::bail!("no transcripts under {}", root.display());
    }

    let mut damaged = 0usize;
    let mut lines = 0usize;
    let mut totals: BTreeMap<&'static str, usize> = BTreeMap::new();

    for path in &files {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        lines += bytes.iter().filter(|&&b| b == b'\n').count();

        // Every file is assumed live. Nothing here can know whether a session is
        // about to be resumed -- sessions never end, they only go quiet -- so
        // the narrow tail allowance is always in force, and it is narrow enough
        // to cost nothing.
        let found = transcript::check(
            &bytes,
            Tail::MayBeIncomplete,
            path.file_stem().and_then(|stem| stem.to_str()),
        );
        if found.is_empty() {
            continue;
        }

        let real: Vec<_> = found.iter().filter(|v| v.rule.is_damage()).collect();
        for violation in &found {
            *totals.entry(violation.rule.name()).or_default() += 1;
        }
        if real.is_empty() {
            if !quiet {
                println!("{} — still being written", path.display());
            }
            continue;
        }
        damaged += 1;
        if quiet {
            continue;
        }

        println!("FAIL {}", path.display());
        let mut by_rule: BTreeMap<&str, Vec<&transcript::Violation>> = BTreeMap::new();
        for violation in real {
            by_rule
                .entry(violation.rule.name())
                .or_default()
                .push(violation);
        }
        for (rule, hits) in by_rule {
            println!("  {rule}: {}", hits.len());
            for hit in hits.iter().take(examples) {
                println!("      line {}: {}", hit.line, hit.detail);
            }
            if hits.len() > examples {
                println!("      ... and {} more", hits.len() - examples);
            }
        }
    }

    println!(
        "\n{} transcript(s), {} lines, {} damaged",
        files.len(),
        lines,
        damaged
    );
    if !totals.is_empty() {
        println!("violations by rule:");
        for (rule, count) in &totals {
            println!("  {count:8}  {rule}");
        }
    }
    if damaged > 0 {
        std::process::exit(1);
    }
    println!("all invariants hold");
    Ok(())
}

/// Every conversation under a directory, including the nested ones a session
/// dispatched.
fn walk(dir: &Path, into: &mut Vec<PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() {
            walk(&path, into)?;
        } else if transcript::is_conversation(&path) {
            into.push(path);
        }
    }
    Ok(())
}
