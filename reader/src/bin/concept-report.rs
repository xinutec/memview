//! The concept census: what lifts, what the lens refused, and what is biggest
//! in the remainder — instrument 2 of `docs/concept-model.md`.
//!
//!     cargo run --release -p reader --bin concept-report -- <corpus.jsonl> [--show <n>] [--sample SUBSTRING]
//!
//! ⚠ **Keyed on the variant PLUS the command, never on the variant alone.** A
//! census keyed on the `Op` variant would rediscover `reading::naming` and read
//! as success (memview#1364, caught before it was built) — `head -5 f` and
//! `cat f` are one `Op::Read` and the `-5` is gone. So the queue's key is the
//! operation's phrase, the unwrapped command name, its subcommand where it has
//! one, and its flags with their values abstracted. **The key errs toward
//! splitting**: a group split in two undercounts a shape, where a wrong merge
//! would rank a shape that does not exist — the same direction every count in
//! this reader errs.
//!
//! ⚠ **Two counts per bucket, and the RANK uses rows.** The reader unrolls
//! loops, so a step count weights a shape by how many iterations ran — measured
//! on the first run: `break` was 112,816 steps out of at most 4,238 rows that
//! mention it, all iteration inflation. A vocabulary is mined from what authors
//! *reach for*, so shapes rank by the rows that hold them, once per row — the
//! same argument `said-report` makes for counting a repeated kind once per
//! call. The step count stays beside it because it is the execution mass, and
//! the balance is stated in steps.
//!
//! ⚠ **The ranking is half the instrument; the diff is the other half.** The
//! automation roadmap is the null hypothesis, never the seed
//! (`docs/concept-model.md`, Decided): read this queue beside the roadmap's
//! target list, and the *disagreement* is the finding — a roadmap item the
//! corpus barely holds, or a top shape no roadmap entry names.
//!
//! Every step is counted exactly once — lifted, refused by name, or queued —
//! and the balance is printed, so a bucket that leaks says so in its own
//! output.

use std::collections::BTreeMap;

use reader::concept::{self, Concept, Why};
use reader::reading::op_name;
use reader::shell_files::{Step, trace};
use reader::shell_ops::{basename, unwrap_command};

/// What one refusal is called on screen and matched against for `--sample`.
fn refusal(why: Why) -> &'static str {
    match why {
        Why::NoLens => "no lens",
        Why::Carrier => "carrier — its work is its children's, counted there",
        Why::NotInPlace => "not in place — prints, a different act",
        Why::Remote => "remote — another machine's world",
        Why::Described => "described subject — a loop's language, unlowerable",
    }
}

/// The concept's name, for the tally. No `_` arm: a new concept must appear.
fn concept_name(concept: &Concept) -> &'static str {
    match concept {
        Concept::Rewrite { .. } => "Rewrite",
        Concept::Page { .. } => "Page",
    }
}

/// The queue's key for one unlifted step.
///
/// The operation's phrase carries the variant *and its discriminating fields*
/// (`transform (in place)` vs `transform`); the command name and subcommand
/// carry what `operands()` drops by construction; the flags carry what no `Op`
/// keeps at all. Values are abstracted — `-5` and `-20` are one `-N`, and
/// `--show=12` is `--show` — because the census ranks *shapes*, and a value in
/// the key would make every occurrence its own group of one.
fn shape(step: &Step) -> String {
    let argv = unwrap_command(&step.argv);
    let Some(head) = argv.first() else {
        // `> /tmp/log` — a line that is a redirection and nothing else.
        return "(a redirection alone)".to_string();
    };
    let op = step
        .op
        .as_ref()
        .map(op_name)
        .unwrap_or("(a redirection alone)");
    let mut words = vec![basename(head).to_string()];
    // A subcommand names the act for the multi-tool commands — `git log` and
    // `git status` are different shapes — and only a word that LOOKS like one
    // joins: wholly lowercase-alphabetic, so a path, a pattern or a sed program
    // stays out of the key.
    if let Some(sub) = argv
        .iter()
        .skip(1)
        .find(|w| !w.starts_with('-'))
        .filter(|w| w.len() >= 2 && w.chars().all(|c| c.is_ascii_lowercase() || c == '-'))
    {
        words.push(sub.clone());
    }
    let mut flags: Vec<String> = argv
        .iter()
        .skip(1)
        .filter(|w| w.starts_with('-') && w.len() > 1)
        .map(|w| {
            let bare = w.split('=').next().unwrap_or(w);
            if bare[1..].chars().all(|c| c.is_ascii_digit()) {
                "-N".to_string()
            } else {
                bare.to_string()
            }
        })
        .collect();
    flags.sort();
    flags.dedup();
    words.extend(flags);
    format!("{op} · {}", words.join(" "))
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
        anyhow::bail!("usage: concept-report <corpus.jsonl> [--show <n>] [--sample SUBSTRING]");
    };
    let show: usize = flag(&args, "--show")
        .and_then(|n| n.parse().ok())
        .unwrap_or(30);
    let sample = flag(&args, "--sample");
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;

    let mut rows = 0usize;
    let mut unparsed = 0usize;
    let mut steps_seen = 0usize;
    let mut lifted: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut refused: BTreeMap<&'static str, usize> = BTreeMap::new();
    // Key → (steps, first witness). The first witness rather than a chosen
    // one: an instrument that picked its own examples would show the reader
    // what it wanted seen.
    let mut queue: BTreeMap<String, (usize, String)> = BTreeMap::new();
    // Bucket → rows holding it, once per row whatever a loop unrolled to.
    let mut rows_of: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        rows += 1;
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        let Ok(script) = reader::project::read(cmd) else {
            unparsed += 1;
            continue;
        };
        let mut here: std::collections::BTreeSet<String> = Default::default();
        for step in trace(&script, cwd, &home).steps {
            steps_seen += 1;
            let bucket = match concept::lift(&step) {
                Ok(concept) => {
                    let name = concept_name(&concept);
                    *lifted.entry(name).or_default() += 1;
                    name.to_string()
                }
                Err(Why::NoLens) => {
                    let key = shape(&step);
                    let entry = queue.entry(key.clone()).or_insert((0, String::new()));
                    entry.0 += 1;
                    if entry.1.is_empty() {
                        entry.1 = truncate(&step.argv.join(" "), 60);
                    }
                    key
                }
                Err(why) => {
                    let name = refusal(why);
                    *refused.entry(name).or_default() += 1;
                    name.to_string()
                }
            };
            if let Some(wanted) = &sample
                && bucket.contains(wanted.as_str())
                && examples.len() < show
            {
                examples.push(truncate(&step.argv.join(" "), 110));
            }
            here.insert(bucket);
        }
        for bucket in here {
            *rows_of.entry(bucket).or_default() += 1;
        }
    }

    println!("{rows} rows, {unparsed} did not parse, {steps_seen} steps from the rest");

    let by_rows = |bucket: &str| rows_of.get(bucket).copied().unwrap_or(0);

    let lifted_total: usize = lifted.values().sum();
    let share = 100.0 * lifted_total as f64 / steps_seen.max(1) as f64;
    println!("\nlifted — {lifted_total} steps ({share:.2}%):        rows    steps");
    for (name, count) in &lifted {
        println!("  {name:<52} {:>8} {count:>8}", by_rows(name));
    }

    println!("\nrefused by the lens, by name:                          rows    steps");
    let refused_total: usize = refused.values().sum();
    let mut named: Vec<_> = refused.iter().collect();
    named.sort_by_key(|(name, _)| std::cmp::Reverse(by_rows(name)));
    for (name, count) in named {
        println!("  {name:<52} {:>8} {count:>8}", by_rows(name));
    }

    println!("\nno lens — the queue, most rows first:                  rows    steps");
    let queued_total: usize = queue.values().map(|(count, _)| count).sum();
    let mut ranked: Vec<_> = queue.iter().collect();
    ranked.sort_by_key(|(key, _)| (std::cmp::Reverse(by_rows(key)), (*key).clone()));
    for (key, (count, witness)) in ranked.iter().take(show) {
        println!("  {key:<52} {:>8} {count:>8}   {witness}", by_rows(key));
    }
    let shown: usize = ranked.iter().take(show).map(|(_, (count, _))| count).sum();
    println!(
        "  … {} further shapes holding {} steps (raise --show to see them)",
        ranked.len().saturating_sub(show),
        queued_total - shown
    );

    // Every step landed in exactly one bucket, or this instrument is lying
    // about one of them.
    let accounted = lifted_total + refused_total + queued_total;
    assert_eq!(steps_seen, accounted, "steps leaked from the census");
    println!(
        "\nbalanced to the unit: {steps_seen} = {lifted_total} lifted + {refused_total} refused + {queued_total} queued"
    );

    if let Some(wanted) = &sample {
        println!("\nwhat `{wanted}` looked like:");
        for cmd in &examples {
            println!("  {cmd}");
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
