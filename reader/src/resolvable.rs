//! Which unnamed subjects a live world could answer at time `t` — which side of
//! *Reading is not running* (`docs/concept-model.md`) each one falls on.
//!
//! ```text
//! $TMPDIR/x            an environment lookup      → answerable
//! *.log at a locus     one readdir                → answerable (and exact)
//! $(git rev-parse …)   running it is the point    → a hole, by decision
//! $1                   what a person will type    → a hole, Why::Outside
//! $f                   bound by the script itself → a hole, and no lookup helps
//! ```
//!
//! ⚠ **This classifies a shape and never asks the world**; asking is the
//! console's job (`docs/reader.md`).
//!
//! ## Why this is not `opaque-shapes`
//!
//! That census cuts the same population by **shape** — locus, language — and
//! two subjects of identical shape fall on opposite sides of this line. One of
//! its buckets held both: `$(cd .. && pwd -P)/dev-lint` fell past a whole-word
//! substitution parse into `BareName`, a label claiming an environment lookup
//! might answer it — **4,118 uses, 74% of the unnamed population**
//! (memview#1445).
//!
//! Hence: the substitution test comes **first and matches anywhere in the
//! word**, and no arm absorbs what the arms above could not read.

/// What a subject the text could not name would take to answer.
///
/// ⚠ **Exactly one variant is answerable, by decision rather than by corpus.**
/// A second means reopening *Reading is not running*, which
/// `docs/concept-model.md` lets a measurement do and convenience not;
/// [`Unnamed::ALL`] and the invariant test make that impossible by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unnamed {
    /// `$TMPDIR`, `$HOME`, `$AMUN_DIR/photos` — a name the session's
    /// environment **may** hold, answered by a lookup that runs nothing.
    ///
    /// ⚠ **An UPPER BOUND: all-uppercase is a convention, not a binding.**
    /// `docs/reader.md` records `A="adb -s host"` and `GEB="ssh -o …"` as
    /// *script* assignments, so some words here are [`Unnamed::ScriptBound`]
    /// wearing the same spelling. Telling them apart needs the script's own
    /// assignments, which a word classifier does not have (memview#1447), and
    /// narrowing by guess — a length floor, an underscore — would be a rule with
    /// no test. Left, named, and printed by the census as "at most", because the
    /// error flatters the resolver.
    Environment,
    /// `$f`, `$d`, `${line}` — bound by the script a few lines above, where no
    /// lookup reaches.
    ///
    /// ⚠ **The split this module exists to make**: identical in spelling to
    /// [`Unnamed::Environment`] and answerable by nothing, so counting the two
    /// together reports a resolver ceiling that does not exist. Reaching here
    /// means the reader could not follow the binding — an undeterminable loop, a
    /// computed assignment.
    ScriptBound,
    /// `$(…)` or a backtick. Running it is the thing prediction precedes, and
    /// no allowlist of "provably pure" spellings survives contact: `$(git
    /// rev-parse HEAD)` and `$(rm -rf x && echo done)` are one shape.
    Substitution,
    /// `$1`, `$2` — a parameter this invocation was handed. `Why::Outside` one
    /// level down, and a hole for the same reason: resolving it would be
    /// inventing the future.
    Positional,
    /// Arithmetic, or a program body offered as a subject. Never a path, so it
    /// is excluded from the population rather than counted as unanswerable —
    /// counting it would inflate the hole side with words that were never
    /// subjects.
    NotASubject,
    /// A shape no rule here recognises.
    ///
    /// ⚠ **Counted as a hole deliberately** — not by doctrine, but because
    /// nothing has shown it answerable, and every refusal in this reader errs
    /// toward undercounting.
    Unclassified,
}

impl Unnamed {
    /// Every variant, so the invariant test cannot miss one added later.
    pub const ALL: [Unnamed; 6] = [
        Unnamed::Environment,
        Unnamed::ScriptBound,
        Unnamed::Substitution,
        Unnamed::Positional,
        Unnamed::NotASubject,
        Unnamed::Unclassified,
    ];

    /// Whether reading the world — an environment lookup, a `readdir`, a
    /// `stat` — would answer this without running any part of the command.
    pub fn answerable(self) -> bool {
        matches!(self, Unnamed::Environment)
    }

    /// Whether it stays a hole: in the population, and not answerable.
    pub fn is_hole(self) -> bool {
        matches!(
            self,
            Unnamed::ScriptBound
                | Unnamed::Substitution
                | Unnamed::Positional
                | Unnamed::Unclassified
        )
    }

    /// Whether it leaves the population entirely, having never been a subject.
    pub fn excluded(self) -> bool {
        matches!(self, Unnamed::NotASubject)
    }

    /// What the census calls it.
    pub fn label(self) -> &'static str {
        match self {
            Unnamed::Environment => "shaped like an environment name — a lookup MAY answer it",
            Unnamed::ScriptBound => "bound by the script itself — no lookup reaches it",
            Unnamed::Substitution => "a substitution — running it is what this precedes",
            Unnamed::Positional => "a positional — what a person will type",
            Unnamed::NotASubject => "never a path subject (arithmetic, a program body)",
            Unnamed::Unclassified => "unrecognised — counted as a hole",
        }
    }
}

/// Classify one unnamed subject, as the text wrote it.
///
/// ⚠ **The order is the whole correctness argument, and two of the four steps
/// are there because getting them wrong has already happened.**
///
/// 1. A word spanning lines is a program body, not a subject.
/// 2. Arithmetic before substitution: `$((300 * i))` contains `$(`, and reading
///    it as a substitution files a non-path as an unanswerable subject.
/// 3. **Substitution anywhere in the word, before any name rule** — the
///    memview#1445 fix. A whole-word parse fails on a trailing `/dev-lint` or a
///    body the corpus truncated, and whatever it drops must not land in a
///    name arm.
/// 4. Digits before capitals: `$1` is vacuously all-uppercase, so an
///    environment rule tested first swallows every positional (`opaque-shapes`
///    filed 84 that way on its first run).
pub fn unnamed(word: &str) -> Unnamed {
    if word.contains('\n') || word.trim_start().starts_with("/*") {
        return Unnamed::NotASubject;
    }
    let word = word.trim();
    if word.contains("$((") {
        return Unnamed::NotASubject;
    }
    if word.contains("$(") || word.contains('`') {
        return Unnamed::Substitution;
    }
    // ⚠ **Every parameter, not the first** (memview#1455). Resolvability across a
    // word is a conjunction: `/tmp/$HOME/$f` is unanswerable because `$f` is,
    // and reading only the first `$` answered `Environment` — the flattering
    // direction, in the function written to stop #1447 flattering the same
    // number. The first part the world cannot answer is what the word is.
    let mut answerable = false;
    for part in word.split('$').skip(1) {
        let name: String = part
            .trim_start_matches('{')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        // `$@`, `$*`, a bare `$`: a expansion this does not model. Unrecognised
        // counts as a hole, so it settles the word.
        if name.is_empty() {
            return Unnamed::Unclassified;
        }
        let part = if name.chars().all(|c| c.is_ascii_digit()) {
            Unnamed::Positional
        } else if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
            Unnamed::Environment
        } else {
            Unnamed::ScriptBound
        };
        if !part.answerable() {
            return part;
        }
        answerable = true;
    }
    if answerable {
        Unnamed::Environment
    } else {
        Unnamed::Unclassified
    }
}
