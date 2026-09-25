//! What the evaluator predicts over the history's commands, and what it does not
//! follow yet — ranked, because that list is what to teach it next.
//!
//!     cargo run --release -p reader --bin predict-report -- <corpus.jsonl> [--show <why> <n>]
//!
//! The history setting: each command is given its text and nothing else, so a
//! write that depends on a file's old contents counts as `not read`. Before a live
//! call the console supplies those, and the same evaluator follows more.
//!
//! Beside the census, the writes the reconstruction (`shell_files`) knows of that
//! the evaluator neither predicted nor refused: the part of the denominator it
//! cannot see yet. `--show unseen <n>` prints them.

use std::collections::{BTreeMap, BTreeSet};

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
    let (mut unseen, mut unseen_commands) = (0usize, 0usize);
    let mut unseen_by: BTreeMap<String, usize> = BTreeMap::new();
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
        // What the reconstruction says the command writes, which the evaluator must
        // either predict or refuse by name. A write it never mentions is neither.
        let mut unseen_here: BTreeSet<String> = BTreeSet::new();
        let mut writers: BTreeSet<String> = BTreeSet::new();
        if let Ok(parsed) = reader::project::read(cmd) {
            let recon = reader::shell_files::extract_knowing(&parsed, Some(cwd), &home, &[]);
            let seen: BTreeSet<&str> = found
                .written
                .iter()
                .map(|w| w.path.as_str())
                .chain(found.unfollowed.iter().filter_map(|u| u.path.as_deref()))
                .collect();
            unseen_here = recon
                .files
                .iter()
                .filter(|f| f.write && !seen.contains(f.path.as_str()))
                .map(|f| f.path.clone())
                .collect();
            writers = recon
                .by_command
                .iter()
                .filter(|(_, (_, w))| *w > 0)
                .map(|(name, _)| name.clone())
                .collect();
        }
        // A refusal without a path — a glob, a variable — may be any of them.
        let unnamed = found.unfollowed.iter().filter(|u| u.path.is_none()).count();
        let unseen_here: Vec<String> = unseen_here.into_iter().skip(unnamed).collect();
        if !unseen_here.is_empty() {
            unseen_commands += 1;
            unseen += unseen_here.len();
            let by = writers.into_iter().collect::<Vec<_>>().join(" ");
            *unseen_by.entry(by.clone()).or_insert(0) += unseen_here.len();
            if let Some((wanted, n)) = &show
                && wanted == "unseen"
                && shown < *n
            {
                shown += 1;
                println!("--- unseen, written by {by}: {unseen_here:?}\n{cmd}\n");
            }
        }
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
    println!(
        "writes never mentioned        {unseen} in {unseen_commands} commands, by the commands writing:"
    );
    let mut ranked: Vec<(String, usize)> = unseen_by.into_iter().collect();
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
