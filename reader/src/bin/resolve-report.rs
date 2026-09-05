//! The resolvability census: how much of what the text could not name could a
//! live world answer at time `t`, and how much stays a hole by decision.
//!
//!     cargo run --release -p reader --bin resolve-report -- <corpus.jsonl> [--show <n>]
//!
//! This is the instrument `docs/concept-model.md`'s dynamic half was missing.
//! The concept census sized the *static* lens by counting what lifts; nothing
//! sized the *dynamic* one, and the number that does is not a lift rate at all
//! — it is the share of unnamed subjects a `readdir` or an environment lookup
//! would resolve, against the share that *Reading is not running* keeps as
//! holes whatever the world says.
//!
//! ⚠ **It reads the same population as `--example opaque-shapes`, on purpose.**
//! Same corpus, same [`Reading`] accounts, so the two are comparable line for
//! line and any disagreement is the *classification* and nothing else. That
//! census cuts by shape — is a locus known, is a language — which is the right
//! question for the static artefact and cannot answer this one: two subjects of
//! identical shape fall on opposite sides of the line here.
//!
//! ⚠ **The headline is a CEILING, never a hit rate.** An environment name is
//! answerable in the sense that a lookup runs no command and returns something;
//! whether this session's environment actually holds it is a fact about a
//! moment, and a prediction is stamped and discarded rather than cached. So
//! this counts what may be asked, which bounds above what may be answered.
//!
//! ⚠ **And it says nothing about whether a resolved prediction is CORRECT.**
//! That is the oracle's job — `reader/tests/oracle.rs` shims `PATH` and can
//! falsify the dynamic claim `S = L ∩ Files(D, t)` exactly, because it is
//! stronger than the static `S ⊆ L` it already checks. A ceiling and a
//! correctness check are different instruments and neither substitutes.

use std::collections::BTreeMap;

use reader::reading::Reading;
use reader::resolvable::{Unnamed, unnamed};

/// Shorten for display, on character boundaries.
fn truncate(s: &str, n: usize) -> String {
    let short: String = s.chars().take(n).collect();
    let short = short.replace('\n', "⏎");
    if s.chars().count() > n {
        short + "…"
    } else {
        short
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let home = std::env::var("HOME").unwrap_or_default();
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(6);
    let path = args
        .get(1)
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| {
            reader::home::cache("bash-corpus.jsonl")
                .to_string_lossy()
                .into_owned()
        });

    let text = std::fs::read_to_string(&path)?;
    let read = Reading::of_corpus(&text, &home)?;

    // uses, distinct, and the first witnesses — kept in the map's own order so
    // the instrument does not choose what it shows.
    //
    // ⚠ **But that order is a SORT here, and memview#1445 is what it cost.**
    // `by_word` is ordered, so `$(` sorts ahead of `$A` and the first four
    // witnesses of a 416-shape bucket were all substitutions — which read as a
    // finding about the bucket and was an artefact of the alphabet. So the
    // witnesses below are labelled as first-by-name, and the COUNTS are what
    // the conclusion rests on.
    let mut by_class: BTreeMap<Unnamed, (usize, usize)> = BTreeMap::new();
    let mut witnesses: BTreeMap<Unnamed, Vec<(String, usize)>> = BTreeMap::new();
    for (word, n) in &read.by_word {
        let class = unnamed(word);
        let entry = by_class.entry(class).or_default();
        entry.0 += n;
        entry.1 += 1;
        let seen = witnesses.entry(class).or_default();
        if seen.len() < show {
            seen.push((word.clone(), *n));
        }
    }

    let words: usize = read.by_word.values().sum();
    // A glob the reader bounded: `S = L ∩ Files(D, t)` is one directory read,
    // and it is the only shape the world answers EXACTLY.
    let bounded: usize = read.by_pattern.values().sum();
    // A locus with an unknown leaf: reading the directory narrows it and does
    // not name it, so it is neither answerable nor a hole.
    let located: usize = read.by_locus.values().sum();

    println!(
        "{} unnamed words ({} distinct) · {bounded} bounded ({} patterns) · {located} located ({} loci)",
        words,
        read.by_word.len(),
        read.by_pattern.len(),
        read.by_locus.len(),
    );

    println!("\nunnamed words, by what would answer them:        uses  distinct");
    let mut ranked: Vec<_> = by_class.iter().collect();
    ranked.sort_by_key(|(class, (uses, _))| (std::cmp::Reverse(*uses), **class));
    for (class, (uses, distinct)) in &ranked {
        println!("  {:<46} {uses:>6} {distinct:>9}", class.label());
        for (word, n) in witnesses.get(class).into_iter().flatten() {
            println!("        {n:>5}  {}", truncate(word, 56));
        }
    }

    let answerable: usize = by_class
        .iter()
        .filter(|(class, _)| class.answerable())
        .map(|(_, (uses, _))| uses)
        .sum();
    let holes: usize = by_class
        .iter()
        .filter(|(class, _)| class.is_hole())
        .map(|(_, (uses, _))| uses)
        .sum();
    let excluded: usize = by_class
        .iter()
        .filter(|(class, _)| class.excluded())
        .map(|(_, (uses, _))| uses)
        .sum();

    // Every unnamed word lands in exactly one of the three, or this instrument
    // is lying about one of them.
    assert_eq!(
        words,
        answerable + holes + excluded,
        "unnamed words leaked from the census"
    );

    // The population the resolver is judged against: everything the text could
    // not resolve to a path, minus what was never a path subject.
    let population = words + bounded + located - excluded;
    let exact = bounded + answerable;
    println!("\nof the {population} subjects the text could not name:");
    // ⚠ **"at most", and the word is load-bearing.** The readdir half is exact
    // — a bounded glob resolves to `S = L ∩ Files(D, t)` and nothing else. The
    // environment half is a SHAPE: the all-uppercase convention cannot tell an
    // exported variable from a script's own `A="adb -s host"`, which this
    // corpus writes constantly (docs/reader.md). So this row is a ceiling that
    // errs toward flattering the resolver, and the honest reading of a fall in
    // it later is that the classifier learned, not that the corpus changed.
    println!(
        "  a read would answer it   {exact:>6}  (at most {:.1}%)   {bounded} by one readdir (exact), at most {answerable} by an environment lookup",
        100.0 * exact as f64 / population.max(1) as f64
    );
    println!(
        "  narrowed, not named      {located:>6}  ({:.1}%)   a locus is known and the leaf is not",
        100.0 * located as f64 / population.max(1) as f64
    );
    println!(
        "  a hole whatever is read  {holes:>6}  ({:.1}%)",
        100.0 * holes as f64 / population.max(1) as f64
    );
    println!("\n⚠ {excluded} were never a path subject and are excluded above.");

    // ⚠ **The one number `docs/concept-model.md` names as able to reopen a
    // decision.** *Reading is not running* says `$(…)` stays a hole, revisable
    // "only by a measurement, not by convenience", and the measurement it names
    // is substitution holes DOMINATING real predictions. So it is printed as
    // its own line, against the hole side rather than the whole population,
    // because that is the comparison the clause asks for.
    let substitution = by_class
        .get(&Unnamed::Substitution)
        .map(|(uses, _)| *uses)
        .unwrap_or(0);
    println!(
        "\nsubstitutions are {:.1}% of the holes ({substitution} of {holes}) — the figure\n\
         `Reading is not running` names as the one that may reopen it.",
        100.0 * substitution as f64 / holes.max(1) as f64
    );
    Ok(())
}
