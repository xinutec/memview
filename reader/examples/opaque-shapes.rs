//! What the unnamed subjects actually ARE, by the thing that generated them.
//!
//!     cargo run --release -p reader --example opaque-shapes -- <corpus.jsonl>
//!
//! `Extract::unnamed` is one bucket of 9,475, and the ranked list makes it
//! obvious it holds at least three different facts: `$(git ls-files)` is a
//! located finite set, `${f%%:*}` is a regular-preserving transformation of
//! another subject, and `+$((BASE + 1))` is arithmetic that is not a path at
//! all. A count over a bucket that mixes those cannot size anything.
//!
//! ⚠ **This classifies the TEXT of a subject, not its meaning.** It is a
//! measurement to decide whether a domain is worth building, not the domain. So
//! every rule is deliberately conservative, an `unclassified` bucket is kept and
//! reported, and each bucket prints samples — a taxonomy nobody can check is a
//! taxonomy that will be wrong quietly.
//!
//! The question it answers: of the subjects the reader cannot name, how many
//! have a **locus** (a directory the answer must live in), how many have a
//! **language** (a shape the answer must match), and how many have neither.

use std::collections::BTreeMap;

use reader::reading::Reading;

/// What produced a subject, and therefore what is knowable about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Shape {
    /// `+$((BASE + 1))`, `$((now - before))` — arithmetic. **Not a path**, and
    /// counting it as an unnamed file subject overstates the gap.
    NotAPath,
    /// A jq filter, a fenced code fragment, a multi-line program body. **Not a
    /// subject at all** — it reached this bucket because something upstream
    /// offered it as one, which is a defect in the reader rather than a gap in
    /// what the text determines.
    NotASubject,
    /// `$1`, `$2` — a positional parameter. Bound by whoever invoked the script,
    /// which no text in the corpus contains.
    Positional,
    /// `Verified/Geo/${s%%:*}`, `/tmp/${f%%:*}.txt` — the DIRECTORY is written
    /// out and only the leaf is unknown. A located language already.
    LocusKnown,
    /// `$(git ls-files)`, `$(find X …)`, `$(ls X)` — not a glob, but a finite
    /// set with a locus the text names.
    LocatedSet,
    /// `${f%%:*}`, `${f%.ts}.js`, `$(basename "$f")` — a regular-preserving
    /// transformation. It has a space exactly when its base does.
    Derived,
    /// `$TMPDIR`, `$HOME`, `$PWD` — an environment variable that names a
    /// directory. Bounded by whatever it held, which the text does not say.
    EnvDir,
    /// `$f`, `$p`, `$d` — a bare name bound by something this text does not
    /// contain. The honest floor.
    BareName,
    /// `$(git rev-parse HEAD)`, `$(date …)` — a substitution whose value could
    /// be anything at all.
    OpaqueRun,
    /// Deliberately kept, and reported. See the module note.
    Unclassified,
}

impl Shape {
    fn label(self) -> &'static str {
        match self {
            Shape::NotAPath => "not a path at all (arithmetic)",
            Shape::NotASubject => "NOT A SUBJECT — a program body, wrongly offered as one",
            Shape::Positional => "a positional parameter ($1, $2)",
            Shape::LocusKnown => "locus known, leaf unknown",
            Shape::LocatedSet => "a located finite set",
            Shape::Derived => "derived from another subject",
            Shape::EnvDir => "an environment directory",
            Shape::BareName => "a bare name, bound elsewhere",
            Shape::OpaqueRun => "a substitution with no locus",
            Shape::Unclassified => "unclassified",
        }
    }

    /// Whether a directory the answer must live in is known from the text.
    fn has_locus(self) -> bool {
        matches!(self, Shape::LocusKnown | Shape::LocatedSet)
    }

    /// Whether a shape the answer must match is known from the text.
    fn has_language(self) -> bool {
        matches!(self, Shape::LocusKnown | Shape::LocatedSet | Shape::Derived)
    }
}

/// The command inside the outermost `$( )`, when the whole subject is one.
fn substitution(word: &str) -> Option<&str> {
    let inner = word.strip_prefix("$(")?.strip_suffix(')')?;
    Some(inner.trim())
}

fn classify(word: &str) -> Shape {
    // ⚠ **Before anything else: a subject that spans lines is not a subject.**
    // jq filters and TypeScript bodies reach this bucket, and classifying them
    // as paths would put a shape on something that never had one.
    if word.contains('\n') || word.trim_start().starts_with("/*") {
        return Shape::NotASubject;
    }
    let word = word.trim();

    // Arithmetic first: `+$((BASE + 1))` also contains `$(`, and reading it as a
    // substitution would file 232 non-paths under a path shape.
    if word.contains("$((") {
        return Shape::NotAPath;
    }

    if let Some(inner) = substitution(word) {
        let head = inner.split_whitespace().next().unwrap_or("");
        let second = inner.split_whitespace().nth(1).unwrap_or("");
        return match (head, second) {
            // The repository is the locus, and `git` names it by being run in it.
            ("git", "ls-files") | ("git", "diff") | ("git", "status") => Shape::LocatedSet,
            // The first operand is the directory it walks.
            ("find", dir) | ("ls", dir) if !dir.is_empty() && !dir.starts_with('-') => {
                Shape::LocatedSet
            }
            ("basename", _) | ("dirname", _) | ("realpath", _) => Shape::Derived,
            _ => Shape::OpaqueRun,
        };
    }

    // A path with directory written out ahead of the first variable:
    // `Verified/Geo/${s%%:*}`, `/tmp/${f%%:*}.txt`, `/Users/me/Code/$p/node_modules`.
    //
    // ⚠ **The variable does not have to be in the LEAF.** The first version of
    // this rule split at the last `/` and required the whole directory to be
    // literal, so `Code/$p/node_modules` — 56 uses, and the largest single shape
    // in the unclassified bucket — was filed as having no locus at all, though
    // `Code` is exactly the directory the answer must live under. Measured
    // 2026-08-23: the leaf-only rule undercounted the locus rate.
    //
    // A word with whitespace in it is not a path here: a one-line jq filter or a
    // template literal can carry both a `/` and a `$`, and widening the rule
    // without this guard would move a program fragment into "locus known" —
    // the direction that flatters the census.
    // ⚠ What the guard costs, so nobody reads the unclassified bucket as a
    // mystery: `/Users/me/Code/scanner/data/$(ls -t …)` has a locus AND a
    // located set, and is refused here for the space inside its `$( )`. Reading
    // that shape properly is the resolver's job, not this census's.
    if !word.contains(char::is_whitespace) {
        let literal = &word[..word.find('$').unwrap_or(word.len())];
        // `cut > 0` keeps the bare root out: `/$p/x` says only that the answer
        // is somewhere on the filesystem, which is not a locus.
        if literal.rfind('/').is_some_and(|cut| cut > 0) {
            return Shape::LocusKnown;
        }
    }

    // `${f%%:*}`, `${f%.ts}.js` — a parameter with an operator applied.
    if word.contains("${") && word.contains(['%', '#', '/']) {
        return Shape::Derived;
    }

    if let Some(name) = word.strip_prefix('$') {
        let name = name.trim_start_matches('{').trim_end_matches('}');
        if name.is_empty() {
            return Shape::Unclassified;
        }
        // ⚠ **Digits first.** `$1` is a positional parameter, and an all-caps
        // test that accepts digits files 84 of them as environment variables —
        // which is what the first run of this did.
        if name.chars().all(|c| c.is_ascii_digit()) {
            return Shape::Positional;
        }
        // An all-caps name is an environment variable by convention, and the
        // ones in this corpus that matter — TMPDIR, HOME, PWD — are directories.
        if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            return Shape::EnvDir;
        }
        return Shape::BareName;
    }

    Shape::Unclassified
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let home = std::env::var("HOME").unwrap_or_default();
    let path = args.get(1).cloned().unwrap_or_else(|| {
        reader::home::cache("bash-corpus.jsonl")
            .to_string_lossy()
            .into_owned()
    });

    let text = std::fs::read_to_string(&path)?;
    let read = Reading::of_corpus(&text, &home)?;

    let mut by_shape: BTreeMap<Shape, (usize, usize)> = BTreeMap::new();
    let mut samples: BTreeMap<Shape, Vec<(String, usize)>> = BTreeMap::new();
    for (word, n) in &read.by_word {
        let shape = classify(word);
        let entry = by_shape.entry(shape).or_default();
        entry.0 += n;
        entry.1 += 1;
        let seen = samples.entry(shape).or_default();
        if seen.len() < 4 {
            seen.push((word.clone(), *n));
        }
    }

    let total: usize = read.by_word.values().sum();
    println!(
        "unnamed by word     {total}  ({} distinct)",
        read.by_word.len()
    );
    println!(
        "  bounded already   {}   ({} distinct absolute patterns)",
        read.by_pattern.values().sum::<usize>(),
        read.by_pattern.len()
    );
    // ⚠ **This census SIZED an opportunity the reader has since taken, and the
    // row it sized has left the population above.** `locus known, leaf unknown`
    // was 612 uses here until memview#1080 taught the walk to resolve them; they
    // now arrive as `Extract::located` and never reach `by_word`. Printed from
    // that map instead, so this table still adds up to the same object and
    // nobody reads a shrunken row as a shrunken problem.
    println!(
        "  located already   {}   ({} distinct loci) — resolved by the reader",
        read.by_locus.values().sum::<usize>(),
        read.by_locus.len()
    );

    println!("\nby what generated them:");
    let mut ranked: Vec<_> = by_shape.iter().collect();
    ranked.sort_by_key(|(shape, (uses, _))| (std::cmp::Reverse(*uses), **shape));
    for (shape, (uses, distinct)) in &ranked {
        println!("  {uses:>6}  {:>4} distinct  {}", distinct, shape.label());
        for (word, n) in samples.get(shape).into_iter().flatten() {
            let short: String = word.chars().take(56).collect();
            println!("            {n:>5}  {}", short.replace('\n', "⏎"));
        }
    }

    // The three numbers the question was asked to settle.
    //
    // ⚠ **`read.by_locus` is added to the locus count and NOT to the total**,
    // because those subjects left `by_word` when the reader learned to resolve
    // them. Leaving it out would report the locus rate falling on the day it
    // was acted on — the flattering direction inverted.
    // ⚠ **Added to the NUMERATORS and to the DENOMINATOR both, or the rate
    // moves for the wrong reason.** `LocusKnown` counted toward a locus and a
    // language and toward `paths`; putting it back in only one of the three
    // would make this report change on the day the subjects did not.
    let resolved_locus: usize = read.by_locus.values().sum();
    let locus: usize = resolved_locus
        + by_shape
            .iter()
            .filter(|(shape, _)| shape.has_locus())
            .map(|(_, (uses, _))| uses)
            .sum::<usize>();
    let language: usize = resolved_locus
        + by_shape
            .iter()
            .filter(|(shape, _)| shape.has_language())
            .map(|(_, (uses, _))| uses)
            .sum::<usize>();
    let not_a_path: usize = [Shape::NotAPath, Shape::NotASubject]
        .iter()
        .filter_map(|shape| by_shape.get(shape).map(|(uses, _)| *uses))
        .sum();
    let paths = (total + resolved_locus).saturating_sub(not_a_path);

    println!("\nof the {paths} that are actually a path subject:");
    println!(
        "  a locus is known  {locus:>6}  ({:.1}%)",
        100.0 * locus as f64 / paths.max(1) as f64
    );
    println!(
        "  a language is too {language:>6}  ({:.1}%)",
        100.0 * language as f64 / paths.max(1) as f64
    );
    println!(
        "  neither           {:>6}  ({:.1}%)",
        paths - language,
        100.0 * (paths - language) as f64 / paths.max(1) as f64
    );
    println!(
        "\n⚠ {not_a_path} were never a path subject at all, and are excluded above —\n           {} arithmetic, {} a program body offered as a subject.",
        by_shape.get(&Shape::NotAPath).map(|(u, _)| *u).unwrap_or(0),
        by_shape
            .get(&Shape::NotASubject)
            .map(|(u, _)| *u)
            .unwrap_or(0),
    );
    Ok(())
}
