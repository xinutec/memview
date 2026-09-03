//! What a command was *for*, as a thing that can be turned back into a command.
//!
//! The first lens of `docs/concept-model.md`. [`crate::shell_ops`] says what a
//! command did to files and [`crate::activity`] names the kind of work; neither
//! can be run backwards, and [`crate::activity`] says so about itself. This is
//! the level that carries enough to regenerate — a **representation** rather
//! than a classification — and the difference is the whole point of it.
//!
//! ## The law
//!
//! ```text
//! lift ∘ lower = id        lowering a concept and lifting it back is identity
//! lower ∘ lift ≠ id        permitted: the text normalises — layout, quoting,
//!                          and here the SPELLING (`sed -i` for `perl -pi`)
//! ```
//!
//! The same shape as the syntax layer's round-trip law, one level up, and with
//! the same constraint carried with it: **the concept is sufficient**. [`lower`]
//! takes a [`Concept`] and nothing else — no `Step`, no source text — because a
//! concept that can only be printed by consulting the command it came from is an
//! annotation, not a concept.
//!
//! ## Why `Rewrite` first, and why it is measured rather than chosen
//!
//! `sed -i 's/a/b/' f` and `perl -pi -e 's/a/b/' f` reach the **identical**
//! [`Op::Transform { program, in_place }`], so two spellings meet in one concept
//! and the cross-language claim has something real to assert. `Page` was the
//! intuitive first pick and is the wrong one: [`Op::Read`] keeps only paths, so
//! `head -5 f` and `cat f` are one key and the range is gone (memview#1364).
//!
//! ⚠ **Shell-only, and that is a correction to the design doc.** The merge one
//! level down is thinner than it claimed: the type both carried readers share is
//! `FileUse { path, write, reached }`, so `python::record` never extracts
//! `re.sub`'s pattern and a Python `Rewrite` would have nothing to compare
//! against. Cross-language needs a parameter field on `program.rs` first.
//!
//! ## Where it attaches
//!
//! [`crate::shell_files::Step`], which is the only place a command and its
//! reading are both in hand. Not "a pure L2 reading", which is what the doc said
//! and what the projection refutes: `operands()` drops flags by construction, so
//! no seed concept's parameters survive at the `Op` alone.

use crate::shell_files::Step;
use crate::shell_ops::Op;

/// A concept's parameter, carrying the precision the reader had and no more.
///
/// ⚠ **The three-part artefact, at this level.** The reader's whole discipline
/// is that a lower bound, a described middle and a counted remainder are
/// different claims; a concept that flattened them to `Option<String>` would
/// throw away the half that is falsifiable. So a subject the text located keeps
/// its locus, a glob keeps its language, and a value that was never in the text
/// stays a [`Subject::Hole`] rather than becoming a guess or a `⊤`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// A path the text determined.
    Named(String),
    /// An unknown member of a known language — `S ⊆ L`, the glob's own pattern.
    Bounded(String),
    /// A directory the answer is rooted at, with the leaf unknown.
    Located(String),
    /// A value that was never in the text: stdin, a parameter, a name bound
    /// outside the program. `program::Why::Outside`, one level up.
    Hole,
}

/// What a command was for.
///
/// One variant today, deliberately. The vocabulary is mined and admitted the way
/// a syntax construct was — biggest first, refused by name until built — and a
/// catch-all `Run { argv }` would take the lift rate to 100% on the first day
/// and mean nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Concept {
    /// A file changed in place by a program applied to its contents.
    ///
    /// `subjects` is a list because the command's own shape is: `sed -i 's/a/b/'
    /// a.ts b.ts` is one act over two files, and splitting it into two concepts
    /// would lower to two commands, which is a different program.
    Rewrite {
        subjects: Vec<Subject>,
        /// The substitution as written — `s/a/b/`. A [`Subject::Hole`]'s
        /// equivalent here is `None`: `sed -i -f fix.sed x` rewrites by a
        /// program that is in another file and not in this text.
        substitution: Option<String>,
    },
}

/// Lift one step into the concept it served, or nothing.
///
/// ⚠ **`None` is the honest answer and must stay cheap to give.** A command with
/// no concept stays an L2/L3 leaf and is counted; that is what keeps a lift rate
/// from being manufactured, and it is the same rule the parser follows when it
/// refuses a construct by name.
pub fn lift(step: &Step) -> Option<Concept> {
    let Some(Op::Transform {
        program,
        program_file,
        paths,
        in_place,
    }) = &step.op
    else {
        return None;
    };
    // ⚠ **Only `-i` is a rewrite.** `sed 's/a/b/' f` prints to stdout and
    // changes nothing — it is a different act with a different concept, and
    // lifting both here would make the lowered form change a file the original
    // did not. The reader already draws this line; this reads it rather than
    // re-deciding it.
    if !in_place {
        return None;
    }
    // ⚠ **A remote step's files are never local**, and a concept that lowered to
    // a bare `sed -i` would claim work on the wrong machine. The step says so;
    // `files` is empty for them by construction, which would otherwise look like
    // a command that named nothing.
    if step.host.is_some() {
        return None;
    }
    let subjects = subjects(step, paths);
    // ⚠ **A DESCRIBED subject cannot be lowered, so it is refused by name.**
    // `Bounded` and `Located` are the reader's middle — `S ⊆ L` at a locus — and
    // no single command spells them: measured 2026-09-03, lowering
    // `/home/…/*.ts` and lifting it back gives [`Subject::Named`], because a
    // pattern written literally in an operand position IS a resolved path to
    // this reader. The language came from a loop, and a loop is not what a
    // `Rewrite` lowers to.
    //
    // Silently keeping them would be worse than dropping them: it turns a
    // described middle into a **false lower bound**, which is the one direction
    // this whole reader refuses. So the lens accepts `Named` and `Hole`, and
    // what it cannot express stays an L2/L3 leaf and is counted — refuse rather
    // than mis-model, the same rule the grammar follows.
    if subjects
        .iter()
        .any(|s| matches!(s, Subject::Bounded(_) | Subject::Located(_)))
    {
        return None;
    }
    Some(Concept::Rewrite {
        subjects,
        // A program given as a file is a hole in the same sense a path is: the
        // substitution exists, and not in this text.
        substitution: program_file.is_none().then(|| program.clone()),
    })
}

/// The subjects, read straight off the step's four accounts.
///
/// ⚠ **`Op::Transform.paths` holds only what RESOLVED**, which is the whole
/// reason this cannot be built from the operation alone: `sed -i 's/a/b/'
/// "$TARGET"` arrives with `paths: []` and the subject in `step.unnamed`, and a
/// lift reading `paths` would report a rewrite of nothing at all. Measured, not
/// assumed — the first version of this did exactly that and the acceptance test
/// for holes caught it.
///
/// ⚠ **Read off the accounts, never re-derived.** The first version matched
/// resolved paths back to their words by comparing leaves, which is a second
/// implementation of resolution and would disagree **silently** the moment a
/// `cd`, a loop variable or a `~` made the word and the path differ — the same
/// argument `shell_files::trace` is built on. The accounts already say which of
/// the three kinds each subject is; this reads them in that order.
///
/// The order is named, described, then counted — the reader's own three-part
/// artefact, so two occurrences of one shape produce the same list.
fn subjects(step: &Step, paths: &[String]) -> Vec<Subject> {
    let mut out: Vec<Subject> = paths.iter().cloned().map(Subject::Named).collect();
    out.extend(step.bounded.iter().cloned().map(Subject::Bounded));
    out.extend(step.located.iter().cloned().map(Subject::Located));
    // Counted, not named: one hole per admission, so a command that could not
    // name two subjects does not lower as though it had one.
    out.extend(step.unnamed.iter().map(|_| Subject::Hole));
    out
}

/// Turn a concept back into a command that does the same thing.
///
/// ⚠ **One canonical spelling, chosen the way the printer chooses quoting.**
/// `sed -i` stands for every in-place transform the lift accepts, so
/// `perl -pi -e` lowers to `sed -i` and the original language survives as
/// provenance rather than as structure — which is what "language choice is
/// spelling" means at this level, and why `lower ∘ lift` is permitted to differ
/// from the text it started from.
///
/// ⚠ **A hole lowers to an unexpanded variable, and that is what a hole IS.**
/// The losslessness claim is that the same holes come back, not fewer — so the
/// spelling has to be one this reader reads back as unnamed. `?` is not:
/// measured, it carries no `/` and no extension, so [`looks_like_path`] refuses
/// it and the subject vanishes entirely, which fails the law. `"$UNNAMED"` is
/// recorded as an admission and lifts back to [`Subject::Hole`].
///
/// ⚠ **An earlier version of this comment said the lowered form was "text a
/// shell would not run, and meant not to be".** That contradicted gate 3, which
/// requires the lowered text to be valid shell, and the law refuted it: a
/// spelling nothing reads back cannot round-trip. What keeps a lowered concept
/// from being mistaken for a script is that its holes are unbound, so a shell
/// given one fails rather than doing something else.
pub fn lower(concept: &Concept) -> String {
    match concept {
        Concept::Rewrite {
            subjects,
            substitution,
        } => {
            let program = substitution.as_deref().unwrap_or("?");
            let words: Vec<String> = subjects.iter().map(spell).collect();
            format!("sed -i '{program}' {}", words.join(" "))
        }
    }
}

/// A subject as the lowered text writes it.
fn spell(subject: &Subject) -> String {
    match subject {
        Subject::Named(path) => path.clone(),
        // The pattern it is a subset of, which is what the reader knows and all
        // it knows: `⟦*.log⟧ = some S ⊆ L(*.log)`.
        Subject::Bounded(pattern) => pattern.clone(),
        // Rooted at this directory, with the leaf unknown.
        Subject::Located(locus) => format!("{}/?", locus.trim_end_matches('/')),
        Subject::Hole => "\"$UNNAMED\"".to_string(),
    }
}
