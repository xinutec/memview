//! The first lens: `Rewrite`, lifted from a step and lowered back to a command.
//!
//! The four gates of `docs/concept-model.md`, on one concept:
//!
//! - **gate 1** — the law: `lift(lower(c)) = c`.
//! - **gate 2** — the level below is the authority: `lower(lift(t))` must read
//!   at L2/L3 identically to `t`, same operation and same subjects. The reader
//!   is to this layer what bash's own printer is to the syntax tree.
//! - **gate 3** — the lowered text is valid shell. Covered by the syntax layer's
//!   own `bash -n` gate, which reads whatever this prints; asserted here only
//!   for the property this layer can break, that a lowered concept **parses**.
//! - **gate 4** — the author's own description, which lives in the corpus and
//!   not in a fixture; `said-report` is where that comparison runs.
//!
//! Plus the two acceptance tests the design named before any of this was built:
//! two languages' spellings must lift equal, and two occurrences differing only
//! in their holes must compare equal — the equality recurrence detection will
//! later stand on.

use reader::concept::{Concept, Subject, Why, lift, lower};
use reader::project::read as parse;
use reader::shell_files::{Step, trace};
use reader::shell_ops::Op;

const HOME: &str = "/home/example";
const CWD: &str = "/home/example/Code/health";

/// Every step one script produced, in order.
fn steps(script: &str) -> Vec<Step> {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    trace(&cmds, Some(CWD), HOME).steps
}

/// The concept one command lifts to, when it lifts to exactly one.
fn only(script: &str) -> Concept {
    let lifted: Vec<Concept> = steps(script)
        .iter()
        .filter_map(|step| lift(step).ok())
        .collect();
    assert_eq!(lifted.len(), 1, "expected one concept from `{script}`");
    lifted.into_iter().next().expect("one")
}

/// What the reader makes of a command, as the pair gate 2 compares.
fn read_as(script: &str) -> Vec<(Option<Op>, Vec<String>)> {
    steps(script)
        .into_iter()
        .map(|step| {
            let mut wrote: Vec<String> = step
                .files
                .iter()
                .filter(|use_| use_.write)
                .map(|use_| use_.path.clone())
                .collect();
            wrote.sort();
            (step.op, wrote)
        })
        .collect()
}

/// ⚠ **Acceptance test 1, and the reason `Rewrite` is the first lens rather
/// than `Page`.** Two languages, one act: the concept is what they have in
/// common, and the spelling is what normalises away.
#[test]
fn two_languages_spelling_one_act_lift_to_the_same_concept() {
    let sed = only("sed -i 's/a/b/' src/geo/velocity.ts");
    let perl = only("perl -pi -e 's/a/b/' src/geo/velocity.ts");

    assert_eq!(sed, perl);
    assert_eq!(
        sed,
        Concept::Rewrite {
            subjects: vec![Subject::Named(
                "/home/example/Code/health/src/geo/velocity.ts".to_string()
            )],
            substitution: Some("s/a/b/".to_string()),
        }
    );
}

/// ⚠ **Acceptance test 2 — the equality recurrence detection stands on.** Two
/// occurrences that differ only in what nobody could name are the *same* work
/// seen twice; if they compared unequal, a shape that recurs across a hundred
/// episodes would look like a hundred distinct things and nothing would ever be
/// found to recur.
#[test]
fn two_occurrences_differing_only_in_their_holes_are_equal() {
    let one = only("sed -i 's/a/b/' \"$TARGET\"");
    let two = only("sed -i 's/a/b/' \"$OTHER\"");

    assert_eq!(
        one,
        Concept::Rewrite {
            subjects: vec![Subject::Hole],
            substitution: Some("s/a/b/".to_string()),
        }
    );
    assert_eq!(one, two);

    // ⚠ And a hole is NOT equal to a name, or the equality would be a way of
    // saying nothing: every rewrite in the corpus would match every other.
    let named = only("sed -i 's/a/b/' src/geo/velocity.ts");
    assert_ne!(one, named);
}

/// **Gate 1 — the law.** `lift(lower(c)) = c` over every shape the lens takes.
#[test]
fn lowering_a_concept_and_lifting_it_back_is_identity() {
    for script in [
        "sed -i 's/a/b/' src/geo/velocity.ts",
        "perl -pi -e 's/x/y/g' src/a.ts src/b.ts",
        "sed -i 's/a/b/' \"$TARGET\"",
    ] {
        let concept = only(script);
        let text = lower(&concept);
        let again = only(&text);
        assert_eq!(concept, again, "lowered `{script}` to `{text}`");
    }
}

/// **Gate 1 again, at the fixpoint.** Lowering twice must give the same text —
/// which is what proves [`lower`] is a function of the concept alone and reads
/// nothing from the command it came from.
#[test]
fn the_lowered_form_is_a_fixpoint() {
    let concept = only("perl -pi -e 's/x/y/g' src/a.ts");
    let once = lower(&concept);
    let twice = lower(&only(&once));
    assert_eq!(once, twice);
}

/// **Gate 2 — the reader below is the authority.** The lowered command must read
/// to the same operation over the same files as the one it came from. A concept
/// that satisfied gate 1 and failed this would be internally consistent and
/// about a different program.
#[test]
fn the_lowered_command_reads_as_the_same_work() {
    let original = "perl -pi -e 's/a/b/' src/geo/velocity.ts";
    let lowered = lower(&only(original));

    assert_eq!(read_as(original), read_as(&lowered));
}

/// **Gate 3 — the lowered text is shell at all.** Our own parser is more
/// permissive than bash in places, so this is the weaker half of that gate; the
/// strong half is `bash -n`, which the syntax layer already runs over anything
/// printed. What this catches is the failure that belongs to *this* layer:
/// a spelling that does not parse.
#[test]
fn the_lowered_text_parses() {
    let text = lower(&Concept::Rewrite {
        subjects: vec![Subject::Named("/tmp/a b.ts".to_string()), Subject::Hole],
        substitution: Some("s/a/b/".to_string()),
    });
    assert!(parse(&text).is_ok(), "did not parse: {text}");
}

/// ⚠ **`sed` without `-i` prints and changes nothing**, so it is a different act
/// and must not lift. Reading both as `Rewrite` would lower to a command that
/// edits a file the original left alone — the direction that invents work.
///
/// And the two refusals are DIFFERENT answers, which is what the census keys
/// on: the printing `sed` was looked at and turned down, where `cat` is simply
/// a shape no lens covers yet.
#[test]
fn a_transform_that_is_not_in_place_is_not_a_rewrite() {
    assert!(
        steps("sed 's/a/b/' src/geo/velocity.ts")
            .iter()
            .all(|step| lift(step) == Err(Why::NotInPlace))
    );
    assert!(
        steps("cat src/geo/velocity.ts")
            .iter()
            .all(|step| lift(step) == Err(Why::NoLens))
    );
}

/// ⚠ **A program in another file is a hole, not an empty substitution.**
/// `sed -i -f fix.sed x` rewrites by something real that is not in this text,
/// and the lowered form has to say so rather than claim the substitution was
/// nothing.
#[test]
fn a_substitution_this_text_does_not_carry_is_a_hole() {
    let concept = only("sed -i -f fix.sed src/geo/velocity.ts");
    let Concept::Rewrite { substitution, .. } = &concept;
    assert_eq!(*substitution, None);
    assert!(lower(&concept).contains('?'), "{}", lower(&concept));
}

/// ⚠ **A remote rewrite is not a local one**, and a concept that lowered to a
/// bare `sed -i` would claim work on this machine. The step knows the host; the
/// lift refuses rather than filing it here.
#[test]
fn a_rewrite_on_another_machine_does_not_lift_as_a_local_one() {
    let steps = steps("ssh amun \"sed -i 's/a/b/' /etc/hosts\"");
    assert!(steps.iter().all(|step| lift(step).is_err()));
    // Two refusals with two names: the `ssh` itself is a carrier whose work is
    // the child's, and the child `sed -i` is refused for WHERE it ran, not for
    // what it is.
    assert!(steps.iter().any(|step| lift(step) == Err(Why::Carrier)));
    assert!(steps.iter().any(|step| lift(step) == Err(Why::Remote)));
}

/// ⚠ **A described subject is REFUSED, and refusing is the finding.**
/// `Bounded` is the reader's middle — an unknown member of a known language —
/// and no single command spells it: lowering `/home/…/*.ts` and lifting it back
/// gives [`Subject::Named`], because a pattern in an operand position IS a
/// resolved path to this reader. The language came from a loop, and a loop is
/// not what a `Rewrite` lowers to.
///
/// Keeping it anyway would turn a described middle into a **false lower bound**,
/// which is the fabrication direction the whole reader refuses. So the first
/// lens accepts named subjects and holes, and this shape stays an L2/L3 leaf to
/// be counted — refuse rather than mis-model.
#[test]
fn a_subject_this_cannot_lower_is_refused_rather_than_flattened() {
    let loop_steps = steps("for f in *.ts; do sed -i 's/a/b/' \"$f\"; done");
    let lifted: Vec<Concept> = loop_steps
        .iter()
        .filter_map(|step| lift(step).ok())
        .collect();

    assert!(
        lifted.is_empty(),
        "a bounded subject must not lift: {lifted:?}"
    );
    // Refused by NAME, not merely missed: this answer is the number that sizes
    // "does `Rewrite` need to lower to a loop" when the census reads it.
    assert!(
        loop_steps
            .iter()
            .any(|step| lift(step) == Err(Why::Described))
    );

    // ⚠ And the reader still HAS the language — nothing was lost below, only
    // left unlifted. A later `Rewrite` that can lower a loop takes it up again.
    assert!(
        loop_steps.iter().any(|step| !step.bounded.is_empty()),
        "the step should still carry the pattern"
    );
}

/// ⚠ **A hole must lower to something this reader reads BACK as a hole**, or
/// the law cannot hold for the commonest unresolvable shape in the corpus.
/// Measured: `?` carries no `/` and no extension, so the path guard refuses it
/// and the subject vanishes — the lowered form would claim a rewrite of nothing.
#[test]
fn a_hole_survives_being_lowered_and_read_again() {
    let concept = only("sed -i 's/a/b/' \"$TARGET\"");
    let text = lower(&concept);

    assert!(text.contains("$UNNAMED"), "{text}");
    assert_eq!(only(&text), concept);
}
