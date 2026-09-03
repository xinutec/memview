//! What the fleet said it was doing, beside what it did.
//!
//!     cargo run --release -p reader --bin said-report -- <corpus.jsonl> <said.jsonl> [--show <n>] [--sample KIND]
//!
//! The first instrument of `docs/concept-model.md`. Every `Bash` call carries a
//! `description` its author wrote at the same moment as the command, and
//! `bash-corpus --said` mines them into a parallel corpus of (command, stated
//! intent) pairs. This is what reads it.
//!
//! ⚠ **The description is a CLAIM about the command, never evidence about what
//! ran.** Nothing here treats it as truth: it is a second, independent reading
//! of the same text — to the concept layer what `declare -f` is to the syntax
//! tree — and the useful output is where it AGREES and where it does not.
//!
//! **The zeroth lift-check.** `reader::activity::Activity` already names what kind of work a
//! command is, from the command alone. Tallying the words a description uses
//! against the kind the reader assigned is the cheapest possible test of whether
//! a concept vocabulary can be mined at all: where one kind is called the same
//! handful of things, the corpus has an idiom to lift; where its words scatter,
//! either the kind is mis-cut or the vocabulary below it is richer than
//! `Activity` — and telling those two apart is the next question, not this one.
//!
//! ⚠ **A high agreement figure is not a result.** `Activity` and the description
//! are both about the same command and both written in ordinary English, so
//! agreement is the null expectation. What is worth reading is the DISAGREEMENT
//! and the concentration: a kind whose top words cover most of it names an
//! idiom, and a word no kind explains is the queue.

use std::collections::BTreeMap;

use reader::shell_files;

/// The stated intents, by the pair that joins them to a corpus row.
///
/// ⚠ **`(at, cmd)` was measured before it was relied on**: over 450,866
/// described calls it resolves to 184,590 distinct keys, and not one of them
/// carries two different descriptions (2026-09-03). A row with no `at` is keyed
/// on the command alone, which is weaker and rare — the corpus carries a stamp
/// wherever the transcript did.
type Said = BTreeMap<(String, String), String>;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let (Some(corpus), Some(said_path)) = (args.get(1), args.get(2)) else {
        anyhow::bail!(
            "usage: said-report <corpus.jsonl> <said.jsonl> [--show <n>] [--sample KIND]"
        );
    };
    let show: usize = flag(&args, "--show")
        .and_then(|n| n.parse().ok())
        .unwrap_or(12);
    let sample = flag(&args, "--sample");
    let home = std::env::var("HOME").unwrap_or_default();

    let said = load_said(said_path)?;
    let text = std::fs::read_to_string(corpus)?;

    let mut commands = 0usize;
    let mut described = 0usize;
    // What each activity kind gets called, and how many commands it holds.
    let mut words: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    let mut per_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut lengths: Vec<usize> = Vec::new();
    let mut examples: Vec<(String, String)> = Vec::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        commands += 1;
        let at = row["at"].as_str().unwrap_or_default();
        let Some(intent) = said.get(&(at.to_string(), cmd.to_string())) else {
            continue;
        };
        described += 1;
        lengths.push(intent.chars().count());

        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        let Ok(script) = reader::project::read(cmd) else {
            continue;
        };
        let found = shell_files::extract_knowing(&script, cwd, &home, &[]);
        // ⚠ **One command can be several activities** — a `&&` chain, a nested
        // shell — and the description is written about the WHOLE call. So the
        // call is filed under each DISTINCT kind of work it did: a chain that
        // greps three times is one `search`, because the sentence beside it is
        // one sentence and counting it thrice would weight a kind by how
        // repetitive its commands are rather than by how often it is described.
        //
        // Taken from the extractor rather than re-derived, for the reason
        // `activity-report` gives: it is the only place an operation and the
        // command it came from are paired, nested shells included.
        let mut kinds: Vec<String> = found
            .activities
            .iter()
            .filter(|kind| kind.is_work())
            .map(|kind| kind.label().to_string())
            .collect();
        kinds.sort();
        kinds.dedup();
        for kind in &kinds {
            *per_kind.entry(kind.clone()).or_default() += 1;
            *words
                .entry(kind.clone())
                .or_default()
                .entry(leading_word(intent))
                .or_default() += 1;
            if sample.as_deref() == Some(kind.as_str()) && examples.len() < show {
                examples.push((intent.clone(), cmd.chars().take(70).collect()));
            }
        }
    }

    let share = 100.0 * described as f64 / commands.max(1) as f64;
    println!("{commands} commands, {described} said what they were for ({share:.1}%)");
    if !lengths.is_empty() {
        lengths.sort_unstable();
        println!(
            "  length in characters: median {}, longest {}",
            lengths[lengths.len() / 2],
            lengths[lengths.len() - 1]
        );
    }

    println!("\nthe zeroth lift-check — what each kind of work gets CALLED:");
    println!("  a kind whose words CONCENTRATE names an idiom; one whose words scatter is");
    println!("  either mis-cut or richer than this vocabulary. Neither is settled here.\n");
    let mut ranked: Vec<_> = per_kind.iter().collect();
    ranked.sort_by_key(|(kind, count)| (std::cmp::Reverse(**count), (*kind).clone()));
    for (kind, count) in ranked.iter().take(show) {
        let said_here = &words[*kind];
        let mut top: Vec<_> = said_here.iter().collect();
        top.sort_by_key(|(word, n)| (std::cmp::Reverse(**n), (*word).clone()));
        let covered: usize = top.iter().take(4).map(|(_, n)| **n).sum();
        let concentration = 100.0 * covered as f64 / (**count).max(1) as f64;
        let named: Vec<String> = top
            .iter()
            .take(4)
            .map(|(word, n)| format!("{word} {n}"))
            .collect();
        println!(
            "  {kind:<16} {count:>7}  ·  {}   [top 4 = {concentration:.0}%, {} distinct words]",
            named.join(" · "),
            said_here.len()
        );
    }

    if let Some(kind) = &sample {
        println!("\nwhat `{kind}` looked like:");
        for (intent, cmd) in &examples {
            println!("  {intent}\n      {cmd}");
        }
    }

    Ok(())
}

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// The first word of a stated intent, lowercased.
///
/// ⚠ **Deliberately crude, and it is an instrument rather than an analysis.**
/// A description is a sentence — *"Run the reader tests"* — and its first word
/// is nearly always the verb, so this is enough to ask whether one kind of work
/// is called the same thing twice. It is NOT a reading of the sentence, and
/// nothing downstream may treat it as one: a concept vocabulary mined from first
/// words would be a vocabulary of English verbs, which is not what
/// `docs/concept-model.md` is asking for.
fn leading_word(said: &str) -> String {
    said.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

fn load_said(path: &str) -> anyhow::Result<Said> {
    let text = std::fs::read_to_string(path)?;
    let mut out = Said::new();
    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let (Some(cmd), Some(intent)) = (row["cmd"].as_str(), row["said"].as_str()) else {
            continue;
        };
        let at = row["at"].as_str().unwrap_or_default();
        out.insert((at.to_string(), cmd.to_string()), intent.to_string());
    }
    Ok(out)
}
