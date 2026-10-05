//! The episode census: which sequences of acts recur across instructions, and
//! whether the author's description sharpens once the sequence is known —
//! instrument 5 of `docs/concept-model.md`.
//!
//!     cargo run --release -p reader --bin episode-report -- <corpus.jsonl> <said.jsonl> [--n 3] [--show 25]
//!
//! The corpus must carry `session` and `turn` (`bash-corpus` since 2026-10-05);
//! an older corpus is refused rather than read as one long episode. The
//! episode is the calls that share a pair, in the order they ran.
//!
//! Three readings, each balanced to the unit so a leak shows in its own output:
//!
//! - **what every step became** — a work token, context, or a carrier — and
//!   every work token's label is a concept or the census's queue key, so the
//!   sequence is total and a gram holds unlifted acts by name rather than
//!   skipping them (which would make `Page | Commit` out of `Page | cargo test |
//!   Commit`);
//! - **the grams that recur**, for each length up to `--n`, ranked by the
//!   EPISODES that hold them, once per episode — the row rule of
//!   `concept-report`, one level up: a shape is what an instruction reached for,
//!   not what a loop unrolled it to;
//! - **the witness**: for each gram, the leading word of the description on the
//!   call its last act came from, beside the same tally for that last act ALONE.
//!   The gate 4 question made measurable: if purpose lives in the episode, the
//!   verb concentrates when the predecessor is known — `History` after `Commit`
//!   says "check" where `History` alone splits — and if it does not, the two
//!   columns read the same and the episode layer has nothing to lift.
//!
//! The leading word is deliberately crude and copied, not shared: `said-report`
//! and `concept-said` each own that question at their own level, and the
//! instruments must be able to disagree about it without one changing the
//! other. It CHECKS a vocabulary and never mines one.

use std::collections::BTreeMap;

use clap::Parser;

use reader::episode::{Call, Token, grams, key, tokens};
use reader::shell_files::trace;

/// The first word of a stated intent, lowercased — the author's own verb.
fn leading_word(said: &str) -> String {
    said.split_whitespace()
        .next()
        .unwrap_or("")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}

/// What one gram's episodes said, by leading word.
#[derive(Default)]
struct Tally {
    episodes: usize,
    calls: usize,
    verbs: BTreeMap<String, usize>,
}

impl Tally {
    /// The top verbs and the share of described calls they cover.
    fn top(&self, n: usize) -> String {
        let described: usize = self.verbs.values().sum();
        if described == 0 {
            return "(no description)".to_string();
        }
        let mut top: Vec<(&String, &usize)> = self.verbs.iter().collect();
        top.sort_by_key(|(v, n)| (std::cmp::Reverse(**n), (*v).clone()));
        top.iter()
            .take(n)
            .map(|(verb, count)| {
                format!("{verb} {:.0}%", 100.0 * **count as f64 / described as f64)
            })
            .collect::<Vec<_>>()
            .join("  ")
    }
}

/// The episode census.
#[derive(Parser)]
struct Cli {
    /// The command corpus, from `bash-corpus`, carrying `session` and `turn`.
    corpus: String,
    /// What the author said each command was for, from `bash-corpus --said`.
    said: String,
    /// The longest gram to count.
    #[arg(long, default_value_t = 3)]
    n: usize,
    /// How many grams of each length to print.
    #[arg(long, default_value_t = 25)]
    show: usize,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let home = std::env::var("HOME").unwrap_or_default();

    let mut said: BTreeMap<(String, String), String> = BTreeMap::new();
    for line in std::fs::read_to_string(&cli.said)?.lines() {
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

    // Episode → its calls, in the order the corpus holds them, which is the
    // transcript's. Keyed on (session, turn); the BTreeMap keeps sessions
    // together and turns in order, which is also the order to print witnesses.
    let mut episodes: BTreeMap<(String, u64), Vec<Call>> = BTreeMap::new();
    let (mut rows, mut unparsed, mut unjoined, mut without_turn) = (0usize, 0usize, 0usize, 0usize);
    let (mut steps_seen, mut work, mut context, mut carriers, mut folded) =
        (0usize, 0usize, 0usize, 0usize, 0usize);
    let (mut lifted, mut queued) = (0usize, 0usize);

    for line in std::fs::read_to_string(&cli.corpus)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        rows += 1;
        let (Some(session), Some(turn)) = (row["session"].as_str(), row["turn"].as_u64()) else {
            without_turn += 1;
            continue;
        };
        let intent = said
            .get(&(
                row["at"].as_str().unwrap_or_default().to_string(),
                cmd.to_string(),
            ))
            .cloned();
        if intent.is_none() {
            unjoined += 1;
        }
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        let Ok(script) = reader::project::read(cmd) else {
            unparsed += 1;
            continue;
        };
        let steps = trace(&script, cwd, &home).steps;
        steps_seen += steps.len();
        let call = tokens(&steps);
        work += call.tokens.len();
        context += call.context;
        carriers += call.carriers;
        folded += call.folded;
        for token in &call.tokens {
            if token.label.contains(" · ") {
                queued += 1;
            } else {
                lifted += 1;
            }
        }
        episodes
            .entry((session.to_string(), turn))
            .or_default()
            .push(Call {
                tokens: call.tokens,
                said: intent,
            });
    }

    if without_turn > 0 && episodes.is_empty() {
        anyhow::bail!(
            "{without_turn} of {rows} rows carry no `session`/`turn` — this corpus predates \
             the episode stamp; re-mine it with `bash-corpus`"
        );
    }

    let calls: usize = episodes.values().map(Vec::len).sum();
    println!(
        "{} episodes holding {calls} calls ({rows} rows; {unparsed} did not parse, {without_turn} carry no turn, {unjoined} have no description)",
        episodes.len()
    );
    assert_eq!(
        steps_seen,
        work + folded + context + carriers,
        "steps leaked from the census"
    );
    println!(
        "{steps_seen} steps = {work} acts ({lifted} lifted, {queued} queued by shape) + {folded} stream acts folded into the act before + {context} context + {carriers} carriers"
    );
    let sizes: Vec<usize> = episodes.values().map(Vec::len).collect();
    let median = {
        let mut s = sizes.clone();
        s.sort_unstable();
        s.get(s.len() / 2).copied().unwrap_or(0)
    };
    let single = sizes.iter().filter(|n| **n == 1).count();
    println!("calls per episode: median {median}, {single} episodes hold one call");

    // n → key → tally. Once per episode: the set of keys an episode holds
    // enters its tally once each, with the descriptions of every call it ended
    // on (a gram an episode repeats contributes each of its descriptions, since
    // each is a separate sentence by the author).
    let mut by_n: Vec<BTreeMap<String, Tally>> = (0..=cli.n).map(|_| BTreeMap::new()).collect();
    for calls in episodes.values() {
        for (n, tallies) in by_n.iter_mut().enumerate().skip(1) {
            let mut here: BTreeMap<String, Vec<Option<&str>>> = BTreeMap::new();
            for (k, intent) in grams(calls, n) {
                here.entry(k).or_default().push(intent);
            }
            for (k, intents) in here {
                let tally = tallies.entry(k).or_default();
                tally.episodes += 1;
                tally.calls += intents.len();
                for intent in intents.into_iter().flatten() {
                    *tally.verbs.entry(leading_word(intent)).or_default() += 1;
                }
            }
        }
    }

    for n in 1..=cli.n {
        let tallies = &by_n[n];
        let total_episodes: usize = tallies.values().map(|t| t.episodes).sum();
        println!(
            "\n{n}-grams — {} distinct, {total_episodes} episode·shapes; by episodes:",
            tallies.len()
        );
        if n == 1 {
            println!(
                "  {:<60} {:>7} {:>7}   the author's verb",
                "act", "episodes", "calls"
            );
        } else {
            println!(
                "  {:<60} {:>7} {:>7}   the author's verb on the last act  ·  that act alone",
                "sequence", "episodes", "calls"
            );
        }
        let mut ranked: Vec<(&String, &Tally)> = tallies.iter().collect();
        ranked.sort_by_key(|(k, t)| (std::cmp::Reverse(t.episodes), (*k).clone()));
        for (k, tally) in ranked.iter().take(cli.show) {
            let shown = truncate(k, 60);
            if n == 1 {
                println!(
                    "  {shown:<60} {:>7} {:>7}   {}",
                    tally.episodes,
                    tally.calls,
                    tally.top(3)
                );
            } else {
                // The last act on its own: re-keyed alone, so its letters start
                // at A as a 1-gram's do.
                let alone = last_alone(k);
                let marginal = by_n[1]
                    .get(&alone)
                    .map(|t| t.top(2))
                    .unwrap_or_else(|| "(not seen alone)".to_string());
                println!(
                    "  {shown:<60} {:>7} {:>7}   {}  ·  {marginal}",
                    tally.episodes,
                    tally.calls,
                    tally.top(3)
                );
            }
        }
    }
    Ok(())
}

/// The last act of a gram key, re-lettered as it would be keyed alone.
///
/// The letters are the LAST parenthesis, and only when it holds letters: a
/// label can carry one of its own (`run python (-c, or a heredoc) · python3`),
/// and splitting at the first put that shape's own acts beside "(not seen
/// alone)" on the first run.
fn last_alone(k: &str) -> String {
    let last = k.rsplit([';', '|']).next().unwrap_or(k).trim();
    let letters_only = |s: &str| {
        !s.is_empty()
            && s.split(',').all(|l| {
                l.chars()
                    .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
            })
    };
    let Some((label, letters)) = last
        .strip_suffix(')')
        .and_then(|s| s.rsplit_once('('))
        .filter(|(_, letters)| letters_only(letters))
    else {
        return last.to_string();
    };
    let mut seen: Vec<&str> = Vec::new();
    let token = Token {
        label: label.to_string(),
        subjects: letters
            .split(',')
            .map(|l| {
                if !seen.contains(&l) {
                    seen.push(l);
                }
                l.to_string()
            })
            .collect(),
    };
    key(&[(&token, false)])
}

/// Shorten for display, on character boundaries.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n - 1).collect::<String>() + "…"
}
