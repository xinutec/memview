//! What the semantics table can read out of the history's shell, and what it
//! cannot.
//!
//!     cargo run --release --bin shell-files -- <corpus.jsonl> [--show <n>] [--paths <n>]
//!
//! The companion to `shell-report`, which measures the *grammar*. This measures
//! the layer above it: of the commands that parse, how many does the table in
//! `shell_files.rs` understand, and which unread commands are the biggest.
//!
//! **Counted per call, not per distinct command** — the opposite of
//! `shell-report`, and deliberately. There, forty runs of one command are one
//! construct to support and counting them forty times would flatter it. Here
//! frequency is the whole signal: a command run four thousand times is worth
//! adding to the table and one run once is not.

use std::collections::BTreeMap;

use memview::shell_ops::{GitOp, Op};
use memview::{shell, shell_files};

/// The shape of an operation, for the distribution.
fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Read { .. } => "read",
        Op::Write { .. } => "write",
        Op::Remove { .. } => "remove",
        Op::Copy { .. } => "copy",
        Op::Move { .. } => "move",
        Op::Search { .. } => "search",
        Op::Transform { in_place: true, .. } => "transform (in place)",
        Op::Transform { .. } => "transform",
        Op::Run { .. } => "run a script",
        Op::Nested { .. } => "open a shell (bash -c, nix --run)",
        Op::ChangeDir { .. } => "cd",
        Op::Git(GitOp::Stage { .. }) => "git stage",
        Op::Git(GitOp::Alter { .. }) => "git alter",
        Op::Git(GitOp::Inspect { .. }) => "git inspect",
        Op::Git(GitOp::Other { .. }) => "git (other)",
        Op::Nothing => "nothing with files",
        Op::Unknown { .. } => "not understood",
    }
}

/// Shorten for display, on character boundaries.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.replace('\n', "⏎");
    }
    s.chars().take(n).collect::<String>().replace('\n', "⏎") + "…"
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: shell-files <corpus.jsonl> [--show <n>] [--paths <n>]");
    };
    let count = |flag: &str, default: usize| -> usize {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .and_then(|n| n.parse().ok())
            .unwrap_or(default)
    };
    let show = count("--show", 25);
    let paths = count("--paths", 0);
    // The check that matters: not how many paths came out, but whether a given
    // one came from a command that really names it. Every doubt about this table
    // has been settled by reading the commands behind a single suspicious path.
    let why = args
        .iter()
        .position(|a| a == "--why")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;
    let mut calls = 0usize;
    let mut unparsed = 0usize;
    let mut handled = 0usize;
    let mut unhandled = 0usize;
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    let mut reads = 0usize;
    let mut writes = 0usize;
    // Distinct paths, so the size of what this produces is visible rather than
    // implied by a total that double-counts every file opened twice.
    let mut distinct: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut witnessed = 0usize;
    // Which commands actually open and change files — the check anyone runs
    // before trusting the table, and the one that showed `sed` to be a pager.
    let mut by_command: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    // What the typed model knows that a path and a direction cannot: the shape
    // of the work, what was being looked for, and what got renamed.
    let mut by_op: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut searched: BTreeMap<String, usize> = BTreeMap::new();
    let mut renames = 0usize;
    let mut nested_unparsed = 0usize;

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
            unparsed += 1;
            continue;
        };
        let found = shell_files::extract(&parsed, cwd, &home);
        handled += found.handled;
        nested_unparsed += found.nested_unparsed;
        for (name, (r, w)) in found.by_command {
            let entry = by_command.entry(name).or_default();
            entry.0 += r;
            entry.1 += w;
        }
        for op in &found.ops {
            *by_op.entry(op_name(op)).or_insert(0) += 1;
            match op {
                Op::Search { pattern, .. } if !pattern.is_empty() => {
                    *searched.entry(pattern.clone()).or_insert(0) += 1;
                }
                Op::Move { .. } => renames += 1,
                _ => {}
            }
        }
        for (name, n) in found.unhandled {
            unhandled += n;
            *by_name.entry(name).or_insert(0) += n;
        }
        for file in found.files {
            if let Some(why) = &why
                && file.path.contains(why.as_str())
                && witnessed < show
            {
                witnessed += 1;
                let mark = if file.write { "write" } else { "read " };
                println!("{mark}  {}\n       {}\n", file.path, cmd.replace('\n', "⏎"));
            }
            let entry = distinct.entry(file.path).or_default();
            if file.write {
                writes += 1;
                entry.1 += 1;
            } else {
                reads += 1;
                entry.0 += 1;
            }
        }
    }

    let commands = handled + unhandled;
    println!("Bash calls          {calls}");
    println!("  unparsed          {unparsed}");
    println!("simple commands     {commands}");
    println!(
        "  understood        {handled}  ({:.1}%)",
        100.0 * handled as f64 / commands.max(1) as f64
    );
    println!("  not in the table  {unhandled}");
    // A wrapper whose inner shell will not parse is a hole in exactly the third
    // of the corpus that runs through one, so it is counted rather than shrugged
    // at — the same rule as every other refusal here.
    println!("  nested, unparsed  {nested_unparsed}");
    println!("file uses           {} reads, {writes} writes", reads);
    println!("distinct paths      {}", distinct.len());

    println!("\nwhat the shell was doing:");
    let mut shapes: Vec<_> = by_op.into_iter().collect();
    shapes.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), *name));
    for (name, n) in shapes {
        println!("  {n:>7}  {name}");
    }
    println!("  {renames:>7}  of those were renames, which no read/write count can express");

    println!("\nmost searched-for terms:");
    let mut terms: Vec<_> = searched.into_iter().collect();
    terms.sort_by_key(|(term, n)| (std::cmp::Reverse(*n), term.clone()));
    for (term, n) in terms.into_iter().take(show) {
        println!("  {n:>7}  {}", truncate(&term, 68));
    }

    println!("\ncommands that CHANGE files, biggest first:");
    let mut writers: Vec<_> = by_command
        .iter()
        .filter(|(_, (_, w))| *w > 0)
        .map(|(name, (r, w))| (name.clone(), *r, *w))
        .collect();
    writers.sort_by_key(|(name, _, w)| (std::cmp::Reverse(*w), name.clone()));
    for (name, reads, writes) in writers.into_iter().take(show) {
        println!("  {writes:>7} writes  {reads:>7} reads   {name}");
    }

    println!("\nunread commands, biggest first:");
    let mut ranked: Vec<_> = by_name.into_iter().collect();
    ranked.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), name.clone()));
    for (name, n) in ranked.into_iter().take(show) {
        println!("  {n:>7}  {name}");
    }

    if paths > 0 {
        println!("\nbusiest paths (reads/writes):");
        let mut ranked: Vec<_> = distinct.into_iter().collect();
        ranked.sort_by_key(|(path, (r, w))| (std::cmp::Reverse(r + w), path.clone()));
        for (path, (r, w)) in ranked.into_iter().take(paths) {
            println!("  {r:>6} {w:>6}  {path}");
        }
    }
    Ok(())
}
