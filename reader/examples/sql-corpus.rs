//! What the SQL reader finds across a whole corpus, and what it still cannot.
//!
//!     cargo run --release -p reader --example sql-corpus -- <corpus.jsonl>
//!
//! The same shape as `python-report` and `javascript-report`: coverage is
//! measured by what was UNDERSTOOD, and the worklist is what to teach it next.
use std::collections::BTreeMap;

use reader::shell_ops::Op;
use reader::{project, shell_files, sql};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args.get(1).cloned().unwrap_or_else(|| {
        reader::home::cache("bash-corpus.jsonl")
            .to_string_lossy()
            .into_owned()
    });
    let home = std::env::var("HOME").unwrap_or_default();
    let text = std::fs::read_to_string(&path)?;

    let (mut calls, mut with_statements, mut silent) = (0usize, 0usize, 0usize);
    let mut reads: BTreeMap<String, usize> = BTreeMap::new();
    let mut writes: BTreeMap<String, usize> = BTreeMap::new();
    let mut verbs: BTreeMap<String, usize> = BTreeMap::new();
    let mut db_files: BTreeMap<String, usize> = BTreeMap::new();
    let mut unreadable: Vec<String> = Vec::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let Ok(parsed) = project::read(cmd) else {
            continue;
        };
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        for op in shell_files::trace(&parsed, cwd, &home).ops {
            let Op::Sql { source, database } = &op else {
                continue;
            };
            calls += 1;
            for db in database {
                *db_files.entry(db.clone()).or_insert(0) += 1;
            }
            if source.trim().is_empty() {
                silent += 1;
                continue;
            }
            let got = sql::read(source);
            if got.is_empty() {
                // Statements were carried and nothing was read out of them —
                // the only bucket that is a defect rather than a fact.
                if unreadable.len() < 5 {
                    unreadable.push(source.chars().take(90).collect());
                }
                continue;
            }
            with_statements += 1;
            for (t, n) in &got.reads {
                *reads.entry(t.clone()).or_insert(0) += n;
            }
            for (t, n) in &got.writes {
                *writes.entry(t.clone()).or_insert(0) += n;
            }
            for (v, n) in &got.verbs {
                *verbs.entry(v.clone()).or_insert(0) += n;
            }
        }
    }

    println!("SQL commands        {calls}");
    println!("  carried statements {with_statements}");
    println!("  carried none       {silent}   (a client opened with no query)");
    println!(
        "  read nothing out of {}   ← the worklist",
        calls - with_statements - silent
    );
    println!(
        "tables              {} read, {} changed  ({} distinct)",
        reads.values().sum::<usize>(),
        writes.values().sum::<usize>(),
        reads
            .keys()
            .chain(writes.keys())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
    );

    let mut ranked: Vec<_> = verbs.iter().collect();
    ranked.sort_by_key(|(v, n)| (std::cmp::Reverse(**n), (*v).clone()));
    println!("\nwhat the SQL was doing:");
    for (v, n) in ranked {
        println!("  {n:>6}  {v}");
    }

    for (title, map) in [("tables read", &reads), ("tables changed", &writes)] {
        let mut ranked: Vec<_> = map.iter().collect();
        ranked.sort_by_key(|(t, n)| (std::cmp::Reverse(**n), (*t).clone()));
        println!("\n{title}, biggest first:");
        for (t, n) in ranked.into_iter().take(12) {
            println!("  {n:>6}  {t}");
        }
    }

    let mut dbs: Vec<_> = db_files.iter().collect();
    dbs.sort_by_key(|(p, n)| (std::cmp::Reverse(**n), (*p).clone()));
    println!("\ndatabase FILES named (sqlite3 only):");
    for (p, n) in dbs.into_iter().take(8) {
        println!("  {n:>6}  {p}");
    }

    if !unreadable.is_empty() {
        println!("\nstatements carried but not read — the worklist:");
        for s in &unreadable {
            println!("  | {}", s.replace('\n', "⏎"));
        }
    }
    Ok(())
}
