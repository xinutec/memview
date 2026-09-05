//! Which unnamed subjects a live world could answer at time `t`.
//!
//! The static reader says what a command *is*; the dynamic half of
//! `docs/concept-model.md` says what it *does, here, now* — the same structure
//! with its holes read off the world. This module is the classifier that sizes
//! that: given a subject the text could not name, which side of *Reading is not
//! running* does it fall on?
//!
//! ```text
//! $TMPDIR/x            an environment lookup      → answerable
//! *.log at a locus     one readdir                → answerable (and exact)
//! $(git rev-parse …)   running it is the point    → a hole, by decision
//! $1                   what a person will type    → a hole, Why::Outside
//! $f                   bound by the script itself → a hole, and no lookup helps
//! ```
//!
//! ⚠ **This library still touches no filesystem.** It classifies a *shape* and
//! says whether the world could be asked; asking is the console's job, which is
//! where `docs/reader.md` put the resolver and why: *"it must live above this
//! library — `reader` touches no filesystem, and that property is worth more
//! than the convenience."*
//!
//! ## Why this is not `opaque-shapes`
//!
//! That census cuts the same population by **shape** — is a locus known, is a
//! language — which is the right question for the static artefact and the wrong
//! one here. Two subjects with identical shape can fall on opposite sides of
//! this line, and one bucket held both: memview#1445. Its `BareName` arm was
//! reached by falling past a whole-word substitution parse, so
//! `$(cd .. && pwd -P)/dev-lint` was counted as *a bare name, bound elsewhere* —
//! a label claiming an environment lookup might answer it. 4,118 uses, 74% of
//! the unnamed population, under a heading asserting the opposite of the truth.
//!
//! So the substitution test comes **first and matches anywhere in the word**,
//! and there is no arm that absorbs what the arms above could not read.

/// What a subject the text could not name would take to answer.
///
/// ⚠ **One variant is answerable and the rest are not, and that asymmetry is a
/// decision rather than an artefact of this corpus.** Adding a second
/// answerable arm means reopening *Reading is not running*, which
/// `docs/concept-model.md` says a measurement may do and convenience may not.
/// [`Unnamed::ALL`] and the invariant test in `reader/tests/resolvable.rs` make
/// that impossible to do by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Unnamed {
    /// `$TMPDIR`, `$HOME`, `$AMUN_DIR/photos` — a name the session's
    /// environment **may** hold, answered by a lookup that runs nothing.
    ///
    /// ⚠ **An UPPER BOUND, and this corpus is why.** The rule is the
    /// all-uppercase convention, and a convention is not a binding: `docs/
    /// reader.md` records `A="adb -s host"` and `GEB="ssh -o …"` as *script*
    /// assignments, and both are all-uppercase. So a word classified here is
    /// one the environment **could** answer, and some of them are
    /// [`Unnamed::ScriptBound`] wearing the same spelling.
    ///
    /// It cannot be decided from the word — it needs the script's own
    /// assignments, which a pure word classifier does not have (memview#1447).
    /// Narrowing the rule by guessing instead — a length floor, an underscore
    /// requirement — would be a rule with no test, which is the disease
    /// memview#1445 is about. So the over-count is **named and left**, and it
    /// runs in the direction that makes the resolver look better than it is,
    /// which is why the census prints this side as "at most".
    Environment,
    /// `$f`, `$d`, `${line}` — a name bound by the script a few lines above.
    ///
    /// ⚠ **The split this module exists to make.** It looks exactly like an
    /// environment name and is answerable by nothing: the binding is inside the
    /// text, and a subject reaching here means the reader could not follow it
    /// (an undeterminable loop, a computed assignment). An environment lookup
    /// will not find it, so counting it beside [`Unnamed::Environment`] reports
    /// a resolver ceiling that does not exist.
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
    /// ⚠ **Counted as a hole, deliberately.** It is not one by doctrine; it is
    /// one because nothing has shown it is answerable, and every refusal in
    /// this reader errs toward undercounting. A census that counted the
    /// unrecognised as resolvable would flatter exactly the number it exists to
    /// size.
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
    let Some(after) = word.split_once('$').map(|(_, rest)| rest) else {
        return Unnamed::Unclassified;
    };
    let name: String = after
        .trim_start_matches('{')
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return Unnamed::Unclassified;
    }
    if name.chars().all(|c| c.is_ascii_digit()) {
        return Unnamed::Positional;
    }
    if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
        return Unnamed::Environment;
    }
    Unnamed::ScriptBound
}
