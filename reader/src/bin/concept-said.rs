//! **Gate 4** — the concept beside the description its author wrote.
//!
//!     cargo run --release -p reader --bin concept-said -- <corpus.jsonl> <said.jsonl> [--show <n>]
//!
//! The other three gates are internal. The law asks whether lifting is
//! invertible, gate 2 asks whether the level below agrees the same files were
//! touched, gate 3 asks whether the lowered text parses — and **a concept that
//! is self-consistent and wrong passes all three.** This is the only one with an
//! outside witness: every `Bash` call carries a description its author wrote at
//! the same moment as the command, and `bash-corpus --said` mines them.
//!
//! ⚠ **The description is a CLAIM about the command, never evidence about what
//! ran.** Nothing here treats it as truth. It is a second, independent reading
//! of the same text, and the useful output is where it agrees and where it does
//! not — the same stance `said-report` takes one level down.
//!
//! ⚠ **Leading words are used to CHECK a vocabulary, never to MINE one.**
//! `said-report::leading_word` carries the warning that a vocabulary mined from
//! first words would be a vocabulary of English verbs, which is not what
//! `docs/concept-model.md` asks for. Checking is the other direction and is
//! sound: the concepts here were mined from the corpus by census, and this asks
//! whether the people running the commands called them the same thing.
//!
//! ⚠ **Agreement is the NULL EXPECTATION and is not the finding.** A `Page` and
//! a description of a page are about the same command and both in ordinary
//! English. What is worth reading is the CONCENTRATION — a concept whose rows
//! share a handful of verbs names an idiom — and the CROSSOVER, where a
//! concept's rows are described with another concept's verbs. The second is the
//! gate: it is what a mislift looks like from outside.

use std::collections::BTreeMap;

use reader::concept::{self, Concept};
use reader::shell_files::trace;

/// The concept's name, for the tally. No `_` arm: a new concept must appear.
fn concept_name(concept: &Concept) -> &'static str {
    match concept {
        Concept::Rewrite { .. } => "Rewrite",
        Concept::Page { .. } => "Page",
        Concept::Search { .. } => "Search",
        Concept::List { .. } => "List",
        Concept::Measure { .. } => "Measure",
        Concept::History { .. } => "History",
        Concept::Status { .. } => "Status",
        Concept::Stage { .. } => "Stage",
        Concept::Commit { .. } => "Commit",
    }
}

/// The first word of a stated intent, lowercased — the author's own verb.
///
/// ⚠ Deliberately crude, and copied rather than shared: `said-report` owns this
/// question one level down and the two instruments must be able to disagree
/// about it without one silently changing the other.
fn leading_word(said: &str) -> String {
    said.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(corpus), Some(said_path)) = (args.get(1), args.get(2)) else {
        anyhow::bail!("usage: concept-said <corpus.jsonl> <said.jsonl> [--show <n>]");
    };
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(6);
    let home = std::env::var("HOME").unwrap_or_default();

    let mut said: BTreeMap<(String, String), String> = BTreeMap::new();
    for line in std::fs::read_to_string(said_path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let (Some(cmd), Some(intent)) = (row["cmd"].as_str(), row["said"].as_str()) else {
            continue;
        };
        said.insert(
            (
                row["at"].as_str().unwrap_or_default().to_string(),
                cmd.to_string(),
            ),
            intent.to_string(),
        );
    }

    // concept → verb → rows
    let mut verbs: BTreeMap<&'static str, BTreeMap<String, usize>> = BTreeMap::new();
    let mut lifted_rows: BTreeMap<&'static str, usize> = BTreeMap::new();
    let (mut joined, mut unjoined, mut no_concept, mut many) = (0usize, 0usize, 0usize, 0usize);

    for line in std::fs::read_to_string(corpus)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let key = (
            row["at"].as_str().unwrap_or_default().to_string(),
            cmd.to_string(),
        );
        let Some(intent) = said.get(&key) else {
            unjoined += 1;
            continue;
        };
        joined += 1;
        let Ok(script) = reader::project::read(cmd) else {
            continue;
        };
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        let steps = trace(&script, cwd, &home).steps;
        let concepts: Vec<Concept> = steps.iter().filter_map(|s| concept::lift(s).ok()).collect();
        // ⚠ **The row must hold exactly one act, not merely one CONCEPT** — and
        // the difference is a defect this instrument shipped with for one run.
        // `git add -A && git status --short` lifts to exactly one concept,
        // because the `git add` refuses; the row was then tallied as a `Status`
        // whose author said "Stage". Same for
        // `git commit … -- paths && git log -1`, tallied as a `History` whose
        // author said "Commit". In both the row's PRINCIPAL act refused and a
        // minor trailing one lifted, so the description was being matched
        // against the least important thing in the row.
        //
        // A work step is one the level below says did something: `echo`, `cd`
        // and `sleep` reach `Op::Nothing` and are not acts. Requiring exactly
        // one means the sentence and the concept are about the same command.
        let work = steps
            .iter()
            .filter(|s| !matches!(s.op, None | Some(reader::shell_ops::Op::Nothing)))
            .count();
        if work != 1 {
            if concepts.is_empty() {
                no_concept += 1;
            } else {
                many += 1;
            }
            continue;
        }
        match concepts.len() {
            0 => no_concept += 1,
            1 => {
                let name = concept_name(&concepts[0]);
                *lifted_rows.entry(name).or_default() += 1;
                *verbs
                    .entry(name)
                    .or_default()
                    .entry(leading_word(intent))
                    .or_default() += 1;
            }
            _ => many += 1,
        }
    }

    println!("joined {joined} rows to a description; {unjoined} had none");
    println!("  of the joined: {no_concept} lift to no concept, {many} hold more than one ACT\n");

    // Which verbs belong to which concept, for the crossover below.
    //
    // ⚠ **By SHARE, never by count — the first version used the count and said
    // nothing.** `Page` is 63,864 rows against `Search`'s 7,690, so it holds
    // more of almost every verb in absolute terms and "owned" them all;
    // `Search`'s own top verb `find` was reported as a crossover into `Page`.
    // That is the instrument measuring which concept is BIGGEST, which is
    // already known. A verb belongs where it is most concentrated: `find` is
    // 17.2% of `Search` and 3.8% of `Page`, so it is `Search`'s.
    let mut owner: BTreeMap<&str, (&'static str, f64)> = BTreeMap::new();
    for (name, tally) in &verbs {
        let rows = lifted_rows[name].max(1) as f64;
        for (verb, n) in tally {
            let share = *n as f64 / rows;
            let entry = owner.entry(verb.as_str()).or_insert((name, 0.0));
            if share > entry.1 {
                *entry = (name, share);
            }
        }
    }

    println!("what the AUTHOR called each concept — top verbs, and how much they cover");
    let mut names: Vec<&&'static str> = lifted_rows.keys().collect();
    names.sort_by_key(|n| std::cmp::Reverse(lifted_rows[*n]));
    for name in names {
        let tally = &verbs[*name];
        let total: usize = tally.values().sum();
        let mut top: Vec<(&String, &usize)> = tally.iter().collect();
        top.sort_by_key(|(v, n)| (std::cmp::Reverse(**n), (*v).clone()));
        let covered: usize = top.iter().take(show).map(|(_, n)| **n).sum();
        println!(
            "\n  {name:8} {total:6} rows   top {show} verbs cover {:.0}%   {} distinct verbs",
            100.0 * covered as f64 / total.max(1) as f64,
            tally.len()
        );
        for (verb, n) in top.iter().take(show) {
            println!("      {n:6}  {verb}");
        }
        // ⚠ **The gate.** A row this lens called X, whose author reached for a
        // verb that belongs to Y. Not proof of a mislift — English is loose, and
        // "check" fits almost anything — but it is the only place an outside
        // witness can contradict the lift at all, and it is where to read.
        let mut crossed: Vec<(&String, &usize, &'static str, f64)> = top
            .iter()
            .filter_map(|(v, n)| match owner.get(v.as_str()) {
                Some((holder, share)) if *holder != *name => Some((*v, *n, *holder, *share)),
                _ => None,
            })
            .collect();
        crossed.sort_by_key(|(_, n, _, _)| std::cmp::Reverse(**n));
        for (verb, n, holder, share) in crossed.iter().take(3) {
            println!(
                "      ⚠ {n:4}  \"{verb}\" — {holder} calls itself that {:.0}% of the time",
                100.0 * share
            );
        }
    }
    Ok(())
}
