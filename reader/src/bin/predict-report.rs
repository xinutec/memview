//! What the evaluator predicts over the history's commands, and what it does not
//! follow yet — ranked, because that list is what to teach it next.
//!
//!     cargo run --release -p reader --bin predict-report -- <corpus.jsonl> [--show <why> <n>]
//!     cargo run --release -p reader --bin predict-report -- --live ~/.console/edits [--show <why|finding> <n>]
//!
//! The history setting: each command is given its text and nothing else, so a
//! write that depends on a file's old contents counts as `not read`. Before a live
//! call the console supplies those, and the same evaluator follows more.
//!
//! Live, the outcome of every file checked: agreed, diverged or never checked,
//! apart for the files predicted on the condition that an unknown program left
//! them alone — and for those, how often each program assumed harmless was.
//!
//! Beside the census, the commands the reconstruction (`shell_files`) knows to
//! write a file and the evaluator neither predicted nor refused — the part of
//! the denominator it cannot see yet. `--show unseen <n>` prints them.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use reader::predict::{Files, Shown, predict};

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
    if path == "--live" {
        let Some(path) = args.get(2) else {
            anyhow::bail!("usage: predict-report --live <edits dir> [--show <why|finding> <n>]");
        };
        return live(Path::new(path), show);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    // `--only <text>`: just the commands containing it.
    let filter = args
        .iter()
        .position(|a| a == "--only")
        .and_then(|at| args.get(at + 1))
        .cloned()
        .unwrap_or_default();

    let mut commands = 0usize;
    let mut unparsed = 0usize;
    let mut writing = 0usize;
    let mut whole = 0usize;
    let mut files = 0usize;
    let mut alternatives = 0usize;
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
        // either predict or refuse by name. Compared per command, not per path: a
        // refusal names one path or none where the reconstruction lists every
        // file a loop touched, so only a command the evaluator says nothing about
        // at all is out of its sight.
        let mut unseen_here: BTreeSet<String> = BTreeSet::new();
        let mut writers: BTreeSet<String> = BTreeSet::new();
        if found.written.is_empty()
            && found.unfollowed.is_empty()
            && let Ok(parsed) = reader::project::read(cmd)
        {
            let recon = reader::shell_files::extract_knowing(&parsed, Some(cwd), &home, &[]);
            unseen_here = recon
                .files
                .iter()
                .filter(|f| f.write)
                .map(|f| f.path.clone())
                .collect();
            writers = recon
                .by_command
                .iter()
                .filter(|(_, (_, w))| *w > 0)
                .map(|(name, _)| name.clone())
                .collect();
        }
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
        alternatives += found.alternatives.len();
        if found.written.is_empty() && found.unfollowed.is_empty() {
            continue;
        }
        if let Some((wanted, n)) = &show
            && wanted == "predicted"
            && shown < *n
            && !found.written.is_empty()
            && cmd.contains(filter.as_str())
        {
            shown += 1;
            println!("--- predicted:\n{cmd}");
            for file in &found.written {
                match &file.text {
                    Some(text) => println!("  {} ⇒\n{text}", file.path),
                    None => println!("  {} ⇒ removed", file.path),
                }
            }
            println!();
        }
        writing += 1;
        files += found.written.len();
        refused += found.unfollowed.len();
        whole += usize::from(found.unfollowed.is_empty());
        for unfollowed in &found.unfollowed {
            let name = unfollowed.why.census_name();
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
    println!("  one of several texts       {alternatives}");
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

/// A live call as the console kept it, with what the evaluator was given.
struct Kept {
    command: String,
    cwd: String,
    shown: Shown,
}

impl Kept {
    /// `None` for a row from before the inputs were kept, which cannot be
    /// predicted again.
    fn read(row: &serde_json::Value) -> Option<Self> {
        Some(Self {
            command: row["command"].as_str()?.to_string(),
            cwd: row["cwd"].as_str()?.to_string(),
            shown: Shown {
                files: serde_json::from_value(row["files"].clone()).ok()?,
                // Rows from before directories were listed have none; one that
                // holds a listing that does not read is not replayable.
                dirs: match &row["dirs"] {
                    serde_json::Value::Null => Default::default(),
                    dirs => serde_json::from_value(dirs.clone()).ok()?,
                },
            },
        })
    }
}

/// The rows of one file under the edits directory, skipping any that do not read.
fn rows(dir: &Path, name: &str) -> Vec<serde_json::Value> {
    std::fs::read_to_string(dir.join(name))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// `--live <edits dir>`: what the console kept from live calls, which had their
/// files read — so `not read` is rare there, and what is left is what to build
/// next. Each refusal and each finding carries the evaluator's inputs, so both
/// are predicted again here under this evaluator: the refusals ranked by what it
/// does not follow today, not by names an older one wrote, and the findings
/// sorted into those still diverging and those it now gets right.
fn live(dir: &Path, show: Option<(String, usize)>) -> anyhow::Result<()> {
    let dir = if dir.is_file() {
        dir.parent().unwrap_or(dir)
    } else {
        dir
    };
    let home = std::env::var("HOME").unwrap_or_default();
    let mut shown = 0usize;

    // Calls with a prediction: one line per call in each session's own file.
    let mut predicted_calls = 0usize;
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".jsonl")
            && !name.ends_with(".diverged.jsonl")
            && ![
                "findings.jsonl",
                "refused.jsonl",
                "outcomes.jsonl",
                "conditional.jsonl",
            ]
            .contains(&name)
        {
            predicted_calls += rows(dir, name).len();
        }
    }

    let (mut refused_calls, mut unreplayable, mut unparsed) = (0usize, 0usize, 0usize);
    let (mut conditional, mut sets) = (0usize, 0usize);
    let mut why: BTreeMap<String, usize> = BTreeMap::new();
    for row in rows(dir, "refused.jsonl") {
        refused_calls += 1;
        let Some(kept) = Kept::read(&row) else {
            unreplayable += 1;
            continue;
        };
        let Ok(script) = reader::syntax::parse(&kept.command) else {
            unparsed += 1;
            continue;
        };
        let found = predict(&script, &kept.cwd, &home, &kept.shown);
        conditional += found.conditional.len();
        sets += found.alternatives.len();
        for unfollowed in &found.unfollowed {
            let name = unfollowed.why.census_name();
            if let Some((wanted, n)) = &show
                && name.contains(wanted.as_str())
                && shown < *n
            {
                shown += 1;
                println!("--- {name}:\n{}\n", kept.command);
            }
            *why.entry(name).or_insert(0) += 1;
        }
    }

    // A finding predicted again: still diverging, agreeing now, or no longer
    // predicted at all — the evaluator refuses today what it once got wrong.
    let (mut findings, mut still, mut agrees, mut unpredicted, mut findings_unreplayable) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    for row in rows(dir, "findings.jsonl") {
        findings += 1;
        let (Some(kept), Some(path)) = (Kept::read(&row), row["path"].as_str()) else {
            findings_unreplayable += 1;
            continue;
        };
        let Ok(script) = reader::syntax::parse(&kept.command) else {
            continue;
        };
        let found = predict(&script, &kept.cwd, &home, &kept.shown);
        let actual: Option<String> = row["actual"].as_str().map(str::to_string);
        let now = found.written.iter().find(|w| w.path == path);
        let outcome = match now {
            None => {
                unpredicted += 1;
                "no longer predicted"
            }
            Some(written) if written.text == actual => {
                agrees += 1;
                "agrees now"
            }
            Some(_) => {
                still += 1;
                "still diverging"
            }
        };
        if let Some((wanted, n)) = &show
            && wanted == "finding"
            && shown < *n
        {
            shown += 1;
            let head: String = kept
                .command
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            println!("--- finding, {outcome}: {path}\n{head}\n");
        }
    }

    println!("live calls with a prediction {predicted_calls}");
    println!("live calls with a refusal    {refused_calls}");
    if unreplayable > 0 {
        println!("  kept before their inputs   {unreplayable}  (not ranked)");
    }
    if unparsed > 0 {
        println!("  not parsed by the tree     {unparsed}  (not ranked)");
    }
    println!("  files predicted if left alone by an unknown program  {conditional}");
    println!("  files one of several texts  {sets}");
    println!("findings                     {findings}");
    println!("  still diverging            {still}");
    println!("  agreeing now               {agrees}");
    println!("  no longer predicted        {unpredicted}");
    if findings_unreplayable > 0 {
        println!("  kept before their inputs   {findings_unreplayable}");
    }
    println!("writes not followed, by why, under this evaluator:");
    let mut ranked: Vec<(String, usize)> = why.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (name, n) in ranked.iter().take(40) {
        println!("  {n:7}  {name}");
    }
    outcomes(&rows(dir, "outcomes.jsonl"));
    Ok(())
}

/// What became of the files checked, certain and conditional apart, and for
/// the conditional ones each assumed program's record.
fn outcomes(rows: &[serde_json::Value]) {
    if rows.is_empty() {
        return;
    }
    // (certain, outcome) -> files
    let mut by: BTreeMap<(bool, String), usize> = BTreeMap::new();
    // program -> (agreed, diverged)
    let mut programs: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for row in rows {
        let outcome = row["outcome"].as_str().unwrap_or("?").to_string();
        let assumed = &row["assumed"];
        *by.entry((assumed.is_null(), outcome.clone())).or_insert(0) += 1;
        let names: BTreeSet<&str> = ["before", "after"]
            .iter()
            .flat_map(|side| assumed[side].as_array().into_iter().flatten())
            .filter_map(serde_json::Value::as_str)
            .collect();
        for name in names {
            let tally = programs.entry(name.to_string()).or_default();
            match outcome.as_str() {
                "agreed" => tally.0 += 1,
                "diverged" => tally.1 += 1,
                _ => {}
            }
        }
    }
    println!("files checked after their call:");
    for ((certain, outcome), n) in &by {
        let kind = if *certain { "certain" } else { "conditional" };
        println!("  {n:7}  {kind} {outcome}");
    }
    if !programs.is_empty() {
        println!("programs assumed to leave files alone, agreed / diverged:");
        let mut ranked: Vec<(String, (usize, usize))> = programs.into_iter().collect();
        ranked.sort_by(|a, b| (b.1.0 + b.1.1).cmp(&(a.1.0 + a.1.1)).then(a.0.cmp(&b.0)));
        for (name, (agreed, diverged)) in ranked.iter().take(40) {
            println!("  {agreed:7} / {diverged:<7}  {name}");
        }
    }
}
