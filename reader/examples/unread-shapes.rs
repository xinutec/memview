//! How an UNREAD command is actually written, in the reader's own accounting.
//!
//!     cargo run --release -p reader --example unread-shapes -- <name> [name…]
//!
//! `--bin shell-files` ranks what the table has no entry for; it cannot show how
//! those calls are spelled, and the spelling is the whole input to writing an
//! entry. A `grep '^lares'` over the corpus is the wrong instrument — it counts
//! the name inside grep patterns and inside pasted source, and a first attempt
//! at this by hand disagreed with the reader by 2x in both directions.
//!
//! So the population comes from `Extract::unhandled` — the same field the rank
//! is built from — and only the TEXT is printed for a human to read.

use std::collections::BTreeMap;

use reader::shell_files;

/// Where `name` appears as a whole word, not inside a longer one.
///
/// Only for choosing what to PRINT — the population is the reader's, and this
/// must never be allowed to become a second way of counting.
fn word_at(text: &str, name: &str) -> Option<usize> {
    let part = |c: char| c.is_alphanumeric() || "_-/.".contains(c);
    let after = |c: char| c.is_alphanumeric() || "_-/".contains(c);
    text.match_indices(name)
        .find(|(at, _)| {
            let before_ok = text[..*at].chars().next_back().is_none_or(|c| !part(c));
            let after_ok = text[at + name.len()..]
                .chars()
                .next()
                .is_none_or(|c| !after(c));
            before_ok && after_ok
        })
        .map(|(at, _)| at)
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let home = std::env::var("HOME").unwrap_or_default();
    let (want, path): (Vec<String>, String) = match args.split_last() {
        Some((last, _)) if last.ends_with(".jsonl") => (
            args[..args.len() - 1].to_vec(),
            args[args.len() - 1].clone(),
        ),
        _ => (args, format!("{home}/.claude/corpus/union.jsonl")),
    };
    anyhow::ensure!(!want.is_empty(), "usage: unread-shapes <name>… [corpus]");

    let text = std::fs::read_to_string(&path)?;
    let mut seen: BTreeMap<String, (usize, BTreeMap<String, usize>)> = BTreeMap::new();
    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let Ok(parsed) = reader::project::read(cmd) else {
            continue;
        };
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        let found = shell_files::extract_knowing(&parsed, cwd, &home, &[]);
        for (name, n) in &found.unhandled {
            if !want.iter().any(|w| w == name) {
                continue;
            }
            let entry = seen.entry(name.clone()).or_default();
            entry.0 += n;
            // The call itself, not the pipeline around it: everything from the
            // name to the first separator, which is what an entry is written
            // against.
            //
            // ⚠ **The name must be found as a WORD, and a plain `find` is not.**
            // The first run of this anchored on the first substring match, so
            // every `ss` sample printed the `ssh` that contained it and every
            // `lares` sample printed the `cd …/lares/rust` — 12 of 12 samples
            // wrong for two of the six names, while the COUNTS beside them were
            // right, because those come from the reader. A display bug that
            // reads as a finding is worse than a crash.
            let at = word_at(cmd, name).unwrap_or(0);
            let tail: String = cmd[at..]
                .split(['|', ';', '\n'])
                .next()
                .unwrap_or("")
                .chars()
                .take(110)
                .collect();
            *entry.1.entry(tail.trim().to_string()).or_insert(0) += 1;
        }
    }

    for (name, (total, shapes)) in &seen {
        println!(
            "=== {name}  {total} calls, {} distinct spellings",
            shapes.len()
        );
        let mut ranked: Vec<_> = shapes.iter().collect();
        ranked.sort_by_key(|(text, n)| (std::cmp::Reverse(**n), (*text).clone()));
        for (text, n) in ranked.iter().take(60) {
            println!("  {n:>5}  {text}");
        }
    }
    Ok(())
}
