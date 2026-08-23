//! How much of the unread list is a function the text DEFINES.
//!
//!     cargo run --release -p reader --example defined-here -- [corpus.jsonl]
//!
//! `probe` sat at 418 calls on the worklist and is not a program: every one of
//! its spellings is `probe() {`, a helper the script defines a few characters
//! earlier. Nobody can write a table entry for it, because it is a different
//! function in every script that declares one — so those calls are work that
//! cannot be done, sitting on a list whose whole purpose is to say what to do
//! next (memview#1124).
//!
//! The question here is only **how big**: what share of `Extract::unhandled` is
//! a name the same command text defines. It is not the fix, and it deliberately
//! does not try to tell a definition from a call to one — that split is the
//! next step, and guessing it here would put a number on the wrong thing.

use std::collections::BTreeMap;

use reader::shell_files;

/// Whether `text` declares `name` as a shell function.
///
/// ⚠ **Both spellings, and the space is not optional to allow.** `probe() {` and
/// `probe () {` are the same declaration and this corpus writes both. A test for
/// the first alone misses the second silently, which is the failure mode this
/// whole probe exists to catch one level up.
fn declares(text: &str, name: &str) -> bool {
    text.match_indices(name).any(|(at, _)| {
        let before_ok = text[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && !"_-/.".contains(c));
        let after = text[at + name.len()..].trim_start();
        before_ok && after.starts_with("()")
    })
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let home = std::env::var("HOME").unwrap_or_default();
    let path = args
        .first()
        .cloned()
        .unwrap_or_else(|| format!("{home}/.claude/corpus/union.jsonl"));

    let text = std::fs::read_to_string(&path)?;
    let mut unread = 0usize;
    let mut defined = 0usize;
    let mut by_name: BTreeMap<String, (usize, usize)> = BTreeMap::new();
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
            unread += n;
            let entry = by_name.entry(name.clone()).or_default();
            entry.0 += n;
            if declares(cmd, name) {
                defined += n;
                entry.1 += n;
            }
        }
    }

    let share = 100.0 * defined as f64 / unread.max(1) as f64;
    println!("unread            {unread}");
    println!("  defined in the same text  {defined}  ({share:.1}%)");
    println!("\nby name, where any call is a locally-defined function:");
    let mut ranked: Vec<_> = by_name
        .iter()
        .filter(|(_, (_, declared))| *declared > 0)
        .collect();
    ranked.sort_by_key(|(name, (_, declared))| (std::cmp::Reverse(*declared), (*name).clone()));
    for (name, (total, declared)) in ranked.iter().take(30) {
        println!("  {declared:>6} of {total:>6}   {name}");
    }
    println!("\n({} distinct names carry at least one)", ranked.len());
    Ok(())
}
