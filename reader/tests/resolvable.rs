//! Which unnamed subjects a live world could answer, and which are holes by
//! decision — the census behind the dynamic resolver in `docs/concept-model.md`.
//!
//! The distinction this pins is the one *Reading is not running* draws: a
//! `readdir`, an environment lookup and a `stat` read state that already
//! exists, and running a command is the thing prediction exists to precede. So
//! `$TMPDIR` is answerable at ask time and `$(git rev-parse HEAD)` is not, and
//! no amount of wanting the second changes which side of that line it is on.
//!
//! ⚠ **A witness copied from a census carries a REAL path, and this repository
//! is public.** The substitution witness below was lifted verbatim from
//! `resolve-report`'s own output and arrived holding a home directory;
//! `DL-TEST-REAL-PATH` caught it at the gate. The corpus is the test suite here
//! by design, so this will recur — sanitise to `/home/example` on the way in,
//! which is the constant `reader/tests/concept.rs` already uses.
//!
//! ⚠ **Every test here is a guard against ONE classifier, in ONE order.** The
//! defect it exists to prevent (memview#1445) was not a missing rule — it was a
//! rule reached too late, so a word that failed to parse as a substitution fell
//! through to the bare-name arm and was counted as answerable. A bucket that
//! absorbs what the rules above it could not read does not announce itself: it
//! prints the same shape of number either way.

use reader::resolvable::{Unnamed, unnamed};

/// ⚠ **The regression this file was written for (memview#1445).** A
/// substitution with anything after its closing paren is still a substitution.
/// `opaque-shapes` tried a whole-word substitution parse, failed on the
/// trailing `/dev-lint`, and filed 4,118 uses under *a bare name, bound
/// elsewhere* — a label asserting the opposite of the truth, since a bare name
/// may resolve from the environment and a substitution never does.
#[test]
fn a_substitution_with_a_tail_is_still_a_substitution() {
    assert_eq!(
        unnamed("$(cd .. && pwd -P)/dev-lint"),
        Unnamed::Substitution
    );
    assert_eq!(
        unnamed("/home/example/Code/scanner/data/$(ls -t /tmp | head -1)"),
        Unnamed::Substitution
    );
    assert_eq!(unnamed("$(basename $f).ts"), Unnamed::Substitution);
}

/// A substitution nobody closed is a substitution too — the corpus carries
/// words truncated mid-command, and reading one as a name is the same
/// fall-through by another route.
#[test]
fn an_unclosed_substitution_does_not_read_as_a_name() {
    assert_eq!(
        unnamed("$(cargo metadata --format-version 1 --no-deps"),
        Unnamed::Substitution
    );
}

/// The old spelling is the same act, and `bash -n` treats it as one.
#[test]
fn a_backtick_is_a_substitution() {
    assert_eq!(unnamed("`git rev-parse HEAD`"), Unnamed::Substitution);
    assert_eq!(unnamed("/tmp/`date +%s`.log"), Unnamed::Substitution);
}

/// ⚠ **Arithmetic contains `$(` and is not a substitution — nor a path.**
/// Tested before the substitution rule, because `$((300 * i))` matches it and
/// filing arithmetic as an unresolvable subject would inflate the hole count
/// with words that were never subjects. `opaque-shapes` gets this order right
/// and it is kept.
#[test]
fn arithmetic_is_not_a_subject_at_all() {
    assert_eq!(unnamed("$((300 * i))"), Unnamed::NotASubject);
    assert_eq!(
        unnamed("sine=frequency=$((300 * i)):sample_rate=48000"),
        Unnamed::NotASubject
    );
}

/// A name the SESSION's environment may hold — the one shape a live world
/// answers with a lookup rather than a run.
#[test]
fn an_environment_name_is_answerable_at_ask_time() {
    assert_eq!(unnamed("$TMPDIR"), Unnamed::Environment);
    assert_eq!(unnamed("${HOME}"), Unnamed::Environment);
    assert_eq!(unnamed("$AMUN_DIR/photos"), Unnamed::Environment);
    assert!(unnamed("$TMPDIR").answerable());
}

/// ⚠ **A lowercase name is bound by the SCRIPT, not by the world, and this is
/// the split the whole census exists to make.** `for f in …; do cat $f; done`
/// reaching here means the reader could not determine the loop — so `$f` is
/// bound a few lines above, where no environment lookup will ever find it.
/// Counting it beside `$TMPDIR` would report a resolver ceiling that does not
/// exist.
#[test]
fn a_script_bound_name_is_not_answerable_however_short_it_is() {
    assert_eq!(unnamed("$f"), Unnamed::ScriptBound);
    assert_eq!(unnamed("$d/gate.json"), Unnamed::ScriptBound);
    assert_eq!(unnamed("${line}"), Unnamed::ScriptBound);
    assert!(!unnamed("$f").answerable());
}

/// ⚠ **The known over-count, pinned so it is read rather than discovered.**
/// The environment arm is the all-uppercase convention, and a convention is not
/// a binding: `docs/reader.md` records `A="adb -s host"` and `GEB="ssh -o …"`
/// as script assignments, both all-uppercase, and this corpus writes that shape
/// constantly. So these words classify as answerable and some of them are not.
///
/// It is left rather than guessed at — a length floor or an underscore
/// requirement would be a rule with no test, which is memview#1445's disease —
/// and the real fix needs the script's own assignments, which a word classifier
/// does not have (memview#1447). The census prints this side as "at most" for
/// exactly this reason. **This test exists so that narrowing the rule is a
/// deliberate act with a measurement behind it, not a tidy-up.**
#[test]
fn an_uppercase_script_variable_is_not_told_apart_from_an_environment_one() {
    assert_eq!(unnamed("$A"), Unnamed::Environment);
    assert_eq!(unnamed("$GEB"), Unnamed::Environment);
    assert!(unnamed("$A").answerable());
}

/// A parameter this invocation was handed. `Why::Outside` at the level below,
/// and it stays a hole for the same reason: the system does not know what a
/// person will type.
#[test]
fn a_positional_stays_a_hole() {
    assert_eq!(unnamed("$1"), Unnamed::Positional);
    assert_eq!(unnamed("${2}"), Unnamed::Positional);
    assert!(!unnamed("$1").answerable());
}

/// ⚠ **Digits before capitals.** `$1` passes an all-uppercase test vacuously —
/// there are no lowercase characters in it — so an environment rule tested
/// first swallows every positional. `opaque-shapes` records having filed 84
/// that way on its first run; the ordering is inherited deliberately.
#[test]
fn a_positional_is_tested_before_the_environment_rule() {
    assert_ne!(unnamed("$1"), Unnamed::Environment);
    assert_ne!(unnamed("$12"), Unnamed::Environment);
}

/// A word with no expansion in it at all reached this census by some other
/// route, and saying so is better than filing it under a shape it does not
/// have.
#[test]
fn a_word_with_no_expansion_is_unclassified_rather_than_guessed() {
    assert_eq!(unnamed("some-file.txt"), Unnamed::Unclassified);
    assert_eq!(unnamed(""), Unnamed::Unclassified);
}

/// A program body offered as a subject is not one, and it is recognised by
/// spanning lines — the guard `opaque-shapes` already carries.
#[test]
fn a_program_body_is_not_a_subject() {
    assert_eq!(unnamed("import json\nprint(1)"), Unnamed::NotASubject);
}

/// ⚠ **The census is only worth its cost if the two sides are disjoint and
/// total.** Every variant answers `answerable()` one way or the other, and a
/// variant added later without a considered answer here fails to compile
/// rather than defaulting to the flattering side.
#[test]
fn every_shape_takes_a_side() {
    for shape in Unnamed::ALL {
        // Exactly one of the two, whatever the shape.
        assert_eq!(shape.answerable(), !shape.is_hole() && !shape.excluded());
    }
    assert_eq!(
        Unnamed::ALL.iter().filter(|s| s.answerable()).count(),
        1,
        "only an environment name is answerable by reading, and adding a second \
         is a doctrine change — see `Reading is not running`"
    );
}
