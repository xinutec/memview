//! What the evaluator predicts over the history's commands, and what it does not
//! follow yet — ranked, because that list is what to teach it next.
//!
//!     cargo run --release -p reader --bin predict-report -- <corpus.jsonl> [--show <why> <n>]
//!
//! The history setting: each command is given its text and nothing else, so a
//! write that depends on a file's old contents counts as `not read`. Before a live
//! call the console supplies those, and the same evaluator follows more.

use std::collections::BTreeMap;

use reader::predict::{Files, Why, predict};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: predict-report <corpus.jsonl> [--show <why> <n>]");
    };
    // The commands behind one reason, printed whole — the check that a reason is
    // what it says.
    let show = args.iter().position(|a| a == "--show").and_then(|at| {
        Some((
            args.get(at + 1)?.clone(),
            args.get(at + 2)?.parse::<usize>().ok()?,
        ))
    });
    let home = std::env::var("HOME").unwrap_or_default();

    let mut commands = 0usize;
    let mut unparsed = 0usize;
    let mut writing = 0usize;
    let mut whole = 0usize;
    let mut files = 0usize;
    let mut refused = 0usize;
    let mut why: BTreeMap<String, usize> = BTreeMap::new();
    let mut shown = 0usize;
    for line in std::fs::read_to_string(path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        commands += 1;
        let Some(cwd) = row["cwd"].as_str().filter(|c| !c.is_empty()) else {
            continue;
        };
        let Ok(script) = reader::syntax::parse(cmd) else {
            unparsed += 1;
            continue;
        };
        let found = predict(&script, cwd, &home, &Files::new());
        if found.written.is_empty() && found.unfollowed.is_empty() {
            continue;
        }
        writing += 1;
        files += found.written.len();
        refused += found.unfollowed.len();
        whole += usize::from(found.unfollowed.is_empty());
        for unfollowed in &found.unfollowed {
            let name = name(&unfollowed.why);
            if let Some((wanted, n)) = &show
                && name.contains(wanted.as_str())
                && shown < *n
            {
                shown += 1;
                println!("--- {name}:\n{cmd}\n");
            }
            *why.entry(name).or_insert(0) += 1;
        }
    }

    println!("commands                     {commands}");
    println!("  not parsed by the tree     {unparsed}");
    println!("commands that write a file   {writing}");
    println!(
        "  every write predicted      {whole}  ({:.1}%)",
        100.0 * whole as f64 / writing.max(1) as f64
    );
    println!("files predicted              {files}");
    println!("writes not followed          {refused}, by why:");
    let mut ranked: Vec<(String, usize)> = why.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (name, n) in ranked.iter().take(40) {
        println!("  {n:7}  {name}");
    }
    Ok(())
}

/// A reason as the report prints it; a program keeps its name, since which one is
/// the worklist.
fn name(why: &Why) -> String {
    match why {
        Why::Program(program) => format!("program {program}"),
        Why::Option(option) => format!("option {option}"),
        other => format!("{other:?}").to_lowercase(),
    }
}
