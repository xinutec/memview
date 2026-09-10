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

use reader::concept::{Concept, Pattern, Quantity, Range, Subject, Why, describe, lift, lower};
use reader::project::read as parse;
use reader::shell_files::{Step, trace};

/// One command's L3 effect reading, as gate 2 compares it: every file use with
/// its direction, the two described accounts, and the count of holes.
type Reading = (Vec<(String, bool)>, Vec<String>, Vec<String>, usize);

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

/// What the reader makes of a command, as the pair gate 2 compares: every
/// file use with its direction, and every account of a subject it could not
/// name.
///
/// ⚠ **The L3 effect reading, deliberately NOT the `Op` variant — recast when
/// `Page` arrived (2026-09-04).** `head -5 f` is `Op::Read` and
/// `sed -n '1,5p' f` is `Op::Transform` that prints: the level below
/// classifies two spellings of one act differently, so holding the lowered
/// form to variant equality would forbid exactly the unification this layer
/// exists to make. What the authority below vouches for is WHAT WAS TOUCHED —
/// each file with its direction — and what it admitted it could not name.
/// `Rewrite` happens to satisfy the stronger variant-equal form; the law does
/// not ask for it.
fn read_as(script: &str) -> Vec<Reading> {
    steps(script)
        .into_iter()
        .map(|step| {
            let mut used: Vec<(String, bool)> = step
                .files
                .iter()
                .map(|use_| (use_.path.clone(), use_.write))
                .collect();
            used.sort();
            (used, step.bounded, step.located, step.unnamed.len())
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

/// ⚠ **Acceptance test 1, for the second lens — and its price is gate 2's
/// recast.** `head -5 f` and `sed -n '1,5p' f` are one act in two spellings,
/// and unlike `Rewrite`'s pair they do NOT meet at one `Op`: the reader below
/// calls one a read and the other a transform that prints. The concept is
/// where they meet — which is what the layer is for — so gate 2's judge is
/// the effect reading, not the variant ([`read_as`] says why).
#[test]
fn two_spellings_of_one_page_lift_to_the_same_concept() {
    let head = only("head -5 src/geo/velocity.ts");
    let sed = only("sed -n '1,5p' src/geo/velocity.ts");

    assert_eq!(head, sed);
    assert_eq!(
        head,
        Concept::Page {
            subjects: vec![Subject::Named(
                "/home/example/Code/health/src/geo/velocity.ts".to_string()
            )],
            range: Range::First(5),
        }
    );
    // And gate 2 for the pair whose spellings cross the L2 variant line: the
    // lowered sed page IS the head page, and both read as the same touch.
    assert_eq!(
        read_as("sed -n '1,5p' src/geo/velocity.ts"),
        read_as(&lower(&sed))
    );
}

/// **Gate 1 over every range shape**, holes and the subjectless page included.
/// A bare `head -50` pages what flows in; the corpus is full of them and the
/// concept says exactly that — no subject, because none was in the text.
#[test]
fn every_range_shape_survives_the_round_trip() {
    for script in [
        "cat src/a.ts src/b.ts",
        "head -5 src/geo/velocity.ts",
        "tail -2 src/geo/velocity.ts",
        "sed -n '25,40p' src/geo/velocity.ts",
        "head -5 \"$TARGET\"",
        "head -50",
    ] {
        let concept = only(script);
        let text = lower(&concept);
        assert_eq!(only(&text), concept, "lowered `{script}` to `{text}`");
    }
}

/// `head f` shows ten lines — POSIX's own default, a documented fact and not
/// a guess — so the concept carries the number the text left implicit.
#[test]
fn a_page_with_no_count_is_the_default_ten() {
    let Concept::Page { range, .. } = only("head src/geo/velocity.ts") else {
        panic!("not a page");
    };
    assert_eq!(range, Range::First(10));
}

/// ⚠ **What looks like a page and is not, refused by name.** `tail -f` waits;
/// `sed '1,5p'` without `-n` prints the WHOLE file and lines 1-5 again;
/// `head -c` counts bytes; `tail -n +2` drops a prefix; `$`-addresses are not
/// digit ranges; `cat -n` numbers its output; `xargs head -5` pages files the
/// PIPE names, none of which the argv spells. Each would lower to a command
/// that does something else. (`wc -l` sat in this list while it measured
/// rather than showed and no lens claimed it; it is the ninth lens's
/// acceptance test now.)
#[test]
fn what_looks_like_a_page_and_is_not_does_not_lift() {
    for script in [
        "tail -f var/log/app.log",
        "sed '1,5p' src/a.ts",
        "head -c 100 src/a.ts",
        "tail -n +2 src/a.ts",
        "sed -n '1,$p' src/a.ts",
        "cat -n src/a.ts",
        "xargs head -5",
    ] {
        assert!(
            steps(script).iter().all(|step| lift(step).is_err()),
            "lifted: {script}"
        );
    }
}

/// ⚠ **A redirect is a subject the argv never spells**, in either direction:
/// `head -5 < f` reads a file no operand names, and `head -5 f > out` writes
/// one. A lowered form built from the concept would silently do less, so both
/// are refused — gate 2 is the reason, applied before the fact.
#[test]
fn a_page_fed_or_captured_by_redirection_does_not_lift() {
    for script in ["head -5 < src/a.ts", "head -5 src/a.ts > /tmp/snippet"] {
        assert!(
            steps(script).iter().all(|step| lift(step).is_err()),
            "lifted: {script}"
        );
    }
}

/// ⚠ **`sed` without `-i` prints and changes nothing**, so it is a different act
/// and must not lift. Reading both as `Rewrite` would lower to a command that
/// edits a file the original left alone — the direction that invents work.
///
/// And the two refusals are DIFFERENT answers, which is what the census keys
/// on: the printing `sed` was looked at and turned down, where `wc` is simply
/// a shape no lens covers.
///
/// ⚠ **`cat` used to be this test's no-lens example and is now a `Page`** — it
/// shows the whole file. The read that still refuses is one that MEASURES
/// rather than shows: `wc -l` counts lines, so no `Page` range describes it.
#[test]
fn a_transform_that_is_not_in_place_is_not_a_rewrite() {
    assert!(
        steps("sed 's/a/b/' src/geo/velocity.ts")
            .iter()
            .all(|step| lift(step) == Err(Why::NotInPlace))
    );
    // A read that reaches no lens stays queued — `du` measures the file
    // rather than its contents, so it is not [`Concept::Measure`] either.
    assert!(
        steps("du -sh src/geo")
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
    let Concept::Rewrite { substitution, .. } = &concept else {
        panic!("not a rewrite: {concept:?}");
    };
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

// ── The card's phrase ────────────────────────────────────────
//
// `describe` is what the ask card renders, and it is held to two properties the
// lowered form is not: it must name every subject the concept carries, and a
// hole must read as a hole. Neither is checked by the round-trip law, because
// the law never looks at this function.

/// ⚠ **The property that matters most on an approval screen.** A hole lowers to
/// `"$UNNAMED"` because the reader must read it back as an admission — and on a
/// card that spelling looks like a variable somebody could go and check. It has
/// to say, in words, that the command touches a file whose name is not in it.
#[test]
fn a_hole_reads_as_a_hole_on_the_card_and_never_as_a_path() {
    let said = describe(&only("sed -i 's/a/b/' \"$TARGET\""));
    assert!(
        said.contains("does not name"),
        "a hole must be stated, got {said:?}"
    );
    assert!(
        !said.contains("UNNAMED") && !said.contains('$'),
        "the lowered spelling must not reach the card, got {said:?}"
    );
}

/// ⚠ **Every subject, never a count.** "2 files" would let the card claim a
/// concept while hiding which files, which is the one thing an approval is for.
#[test]
fn the_phrase_names_every_subject_the_concept_carries() {
    let said = describe(&only("sed -i 's/a/b/' one.ts two.ts"));
    assert!(
        said.contains("one.ts") && said.contains("two.ts"),
        "{said:?}"
    );
}

/// The whole point of the layer, said on the card: one act, two spellings, one
/// sentence. If these ever diverge the card is reporting spelling again.
///
/// ⚠ **And the path is the RESOLVED one, not the word that was typed.** This
/// test first expected `notes.md`, the reader answered
/// `/home/example/Code/health/notes.md`, and the reader was right.
/// [`Step::argv`] is "the words as the shell would have run them" for exactly
/// this reason, and on an approval screen it is the difference that decides:
/// a relative path hides WHICH file, and the working directory it resolves
/// against is the one thing a person cannot see by reading the command.
#[test]
fn two_spellings_of_one_page_describe_identically_and_name_the_resolved_path() {
    let a = describe(&only("head -5 notes.md"));
    let b = describe(&only("sed -n '1,5p' notes.md"));
    assert_eq!(a, b);
    assert_eq!(a, format!("Show the first 5 lines of {CWD}/notes.md"));
}

/// A stream page named no file, and inventing one would be the fabrication this
/// tower refuses at every level.
#[test]
fn a_stream_page_says_it_was_given_its_input() {
    assert_eq!(
        describe(&only("head -50")),
        "Show the first 50 lines of what it is given"
    );
}

/// ⚠ **Acceptance test 1, for the third lens.** `egrep` IS `grep -E`, so two
/// spellings of one act must lift equal and the dialect must survive as a field
/// rather than as the program's name.
#[test]
fn two_spellings_of_one_search_lift_to_the_same_concept() {
    let egrep = only("egrep 'a|b' src/geo/velocity.ts");
    let grep = only("grep -E 'a|b' src/geo/velocity.ts");

    assert_eq!(egrep, grep);
    assert_eq!(
        grep,
        Concept::Search {
            subjects: vec![Subject::Named(
                "/home/example/Code/health/src/geo/velocity.ts".to_string()
            )],
            pattern: Pattern::Extended("a|b".to_string()),
            fold_case: false,
            descend: false,
        }
    );
}

/// ⚠ **And a dialect is MEANING, so the two must NOT compare equal.** `a|b` is
/// three literal characters to basic grep and an alternation to `-E` — measured
/// in bash, both. A lens that dropped the dialect would make these one concept
/// and lower it to a command matching different lines.
#[test]
fn a_search_dialect_is_not_spelling() {
    let basic = only("grep 'a|b' src/geo/velocity.ts");
    let extended = only("grep -E 'a|b' src/geo/velocity.ts");
    assert_ne!(basic, extended);

    let fixed = only("fgrep 'a|b' src/geo/velocity.ts");
    assert_ne!(basic, fixed);
    assert_ne!(extended, fixed);
}

/// **Gate 1 over every search shape**, holes and the subjectless stream
/// included. `-n` normalises away because it decorates the same lines; `-i` and
/// `-r` do not, because they change which lines come back.
#[test]
fn every_search_shape_survives_the_round_trip() {
    for script in [
        "grep foo src/a.ts",
        "grep -n foo src/a.ts src/b.ts",
        "grep -E '^(a|b)$' src/a.ts",
        "fgrep 'a.b' src/a.ts",
        "grep -i foo src/a.ts",
        "grep -rn foo src/geo",
        "grep -in foo \"$TARGET\"",
        "grep -n foo",
    ] {
        let concept = only(script);
        let text = lower(&concept);
        assert_eq!(only(&text), concept, "lowered `{script}` to `{text}`");
    }
}

/// **Gate 2 for the third lens** — the lowered command reads as the same work
/// over the same files.
///
/// **Gate 3** rides along: the lowered text has to parse, and a pattern holding
/// `|` and a space is exactly what would break it if the quoting were dropped.
#[test]
fn a_lowered_search_reads_as_the_same_work_and_parses() {
    let original = "egrep -n 'a|b c' src/geo/velocity.ts";
    let lowered = lower(&only(original));

    assert_eq!(read_as(original), read_as(&lowered));
    assert!(parse(&lowered).is_ok(), "did not parse: {lowered}");
    assert_eq!(
        lowered,
        "grep -E 'a|b c' /home/example/Code/health/src/geo/velocity.ts"
    );
}

/// ⚠ **What scans like a search and answers a different question, refused BY
/// NAME so the census can size each one.** These are not gaps — every one is a
/// design question with a row count behind it, and flattening any of them would
/// have the concept claim lines the command never printed.
#[test]
fn a_search_with_another_product_refuses_by_name() {
    for (script, why) in [
        ("grep -c foo src/a.ts", Why::NotLines),
        ("grep -l foo src/a.ts", Why::NotLines),
        ("grep -q foo src/a.ts", Why::NotLines),
        ("grep -o foo src/a.ts", Why::NotLines),
        ("grep -v foo src/a.ts", Why::Inverted),
        ("grep -A 5 foo src/a.ts", Why::WithContext),
        ("grep -B6 foo src/a.ts", Why::WithContext),
        ("grep -m1 foo src/a.ts", Why::WithContext),
        ("grep -rn --include=*.ts foo src", Why::Filtered),
        ("grep -e foo src/a.ts", Why::PatternInFlag),
        ("rg -n foo src/a.ts", Why::NoLens),
        // ⚠ The level below drops a bare word rather than guess what it is,
        // so the subjects come back empty — the same shape a pipe produces.
        ("grep -rn foo src", Why::UnreadSubject),
        ("grep -n foo notes", Why::UnreadSubject),
    ] {
        let found: Vec<Why> = steps(script)
            .iter()
            .filter_map(|step| lift(step).err())
            .collect();
        assert!(
            found.contains(&why),
            "`{script}` should refuse {why:?}, got {found:?}"
        );
    }
}

/// ⚠ **A redirect is a subject the argv never spells**, and the guard `Page`
/// carries had to be shared rather than repeated — a search writing its hits to
/// a file would otherwise lower to one that prints them instead.
#[test]
fn a_search_that_redirects_or_reads_a_stream_does_not_lift() {
    for script in [
        "grep -n foo src/a.ts > hits.txt",
        "grep -n foo < src/a.ts",
        "xargs grep -n foo",
    ] {
        let lifted: Vec<Concept> = steps(script)
            .iter()
            .filter_map(|step| lift(step).ok())
            .collect();
        assert!(lifted.is_empty(), "`{script}` lifted to {lifted:?}");
    }
}

/// The card sentence, which is what a person approving actually reads. A
/// recursive search says so, because "in src" and "everything under src" are
/// different amounts of machine to touch.
#[test]
fn a_search_describes_what_it_looks_for_and_where() {
    assert_eq!(
        describe(&only("grep -n foo notes.md")),
        format!("Find lines matching foo in {CWD}/notes.md")
    );
    assert_eq!(
        describe(&only("grep -rin foo src/geo")),
        format!("Find lines matching foo in everything under {CWD}/src/geo, ignoring case")
    );
    assert_eq!(
        describe(&only("grep -n foo")),
        "Find lines matching foo in what it is given"
    );
}

/// ⚠ **The same trap the `Search` lens found, asked of `Page`.** A bare word is
/// dropped by the level below rather than guessed at, so `cat notes` would come
/// back with no subjects — which is the shape `… | cat` produces, and would have
/// the card say "what it is given" about a file the text named.
#[test]
fn a_page_naming_an_operand_the_reader_cannot_resolve_does_not_lift() {
    for script in [
        "cat notes",
        "head -5 notes",
        "tail -n 3 notes",
        "sed -n '1,5p' notes",
    ] {
        let lifted: Vec<Concept> = steps(script)
            .iter()
            .filter_map(|step| lift(step).ok())
            .collect();
        assert!(
            lifted.is_empty(),
            "`{script}` lifted to {lifted:?} — it names an operand nothing resolved"
        );
    }
}

/// **Gate 1 over every listing shape.** `-R` and `-a` are carried because both
/// change which names come back; nothing else is.
#[test]
fn every_listing_shape_survives_the_round_trip() {
    for script in [
        "ls src/geo",
        "ls -R src/geo",
        "ls -a src/geo",
        "ls -Ra src/geo",
        "ls src/geo src/a.ts",
    ] {
        let concept = only(script);
        let text = lower(&concept);
        assert_eq!(only(&text), concept, "lowered `{script}` to `{text}`");
    }
}

/// ⚠ **The listing boundary, refused BY NAME so the census sizes each.**
///
/// `ls -l` hands back mode, size and time — the same locus, a different product,
/// which is the call `grep -c` gets. `ls -d` NAMES the directory instead of
/// enumerating it, inverting the act rather than adjusting it. Bare `ls` has a
/// real locus the text never wrote, and inventing it is the fabrication this
/// layer refuses everywhere.
///
/// ⚠ **`find` is refused WHOLE, and the census is the argument.** Its operands
/// are a predicate expression — measured 2026-09-10, 1,277 rows use `-o`, 1,242
/// `-not`, 281 `-prune` — so keeping only the `-name` value would claim a
/// NARROWER walk than the command made. That is a false lower bound.
#[test]
fn what_looks_like_a_listing_and_is_not_refuses_by_name() {
    for (script, why) in [
        ("ls -l src/geo", Why::WithMetadata),
        ("ls -la src/geo", Why::WithMetadata),
        ("ls -d src/geo", Why::NotTheContents),
        ("ls", Why::ImplicitLocus),
        ("find src/geo -name '*.ts'", Why::Predicate),
        ("find src/geo -name a -o -name b", Why::Predicate),
        ("ls notes", Why::UnreadSubject),
    ] {
        let found: Vec<Why> = steps(script)
            .iter()
            .filter_map(|step| lift(step).err())
            .collect();
        assert!(
            found.contains(&why),
            "`{script}` should refuse {why:?}, got {found:?}"
        );
    }
}

/// ⚠ **The three readers under `Op::Read` must not claim each other's
/// commands.** They are asked in order — page, listing, measure — and each
/// declining is what hands the step on. Only a [`Why::NoLens`] falls through:
/// a named refusal (`find`, `ls -l`) is an answer, not a hand-off.
#[test]
fn a_page_a_listing_and_a_measure_do_not_claim_each_others_commands() {
    assert!(matches!(only("cat src/a.ts"), Concept::Page { .. }));
    assert!(matches!(only("ls src/geo"), Concept::List { .. }));
    assert!(matches!(only("wc -l src/a.ts"), Concept::Measure { .. }));
}

/// ⚠ **Acceptance test for the ninth lens.** The locus `cat` shows, read for
/// one number — and the FLAG is the product: `-l` and `-c` are different
/// questions with different answers, so the quantity is carried, canonically
/// spelled on the way back down, and said in words on the card.
#[test]
fn a_count_of_lines_lifts_lowers_and_reads_back() {
    let original = "wc -l src/a.ts";
    let concept = only(original);
    assert_eq!(
        concept,
        Concept::Measure {
            subjects: vec![Subject::Named(format!("{CWD}/src/a.ts"))],
            quantity: Quantity::Lines,
        }
    );
    let lowered = lower(&concept);
    assert_eq!(read_as(original), read_as(&lowered));
    assert!(parse(&lowered).is_ok(), "did not parse: {lowered}");
    assert_eq!(lowered, format!("wc -l {CWD}/src/a.ts"));
    // Gate 1: the lowered text lifts back to the same concept.
    assert_eq!(only(&lowered), concept);
    assert_eq!(
        describe(&concept),
        format!("Count the lines of {CWD}/src/a.ts")
    );
}

/// `-c` is BYTES, and the card says so — softening it to "characters" would
/// blur the one distinction the `-c`/`-m` pair exists to draw.
#[test]
fn a_count_of_bytes_says_bytes() {
    let concept = only("wc -c src/a.ts");
    assert!(matches!(
        concept,
        Concept::Measure {
            quantity: Quantity::Bytes,
            ..
        }
    ));
    assert_eq!(lower(&concept), format!("wc -c {CWD}/src/a.ts"));
    assert_eq!(
        describe(&concept),
        format!("Count the bytes of {CWD}/src/a.ts")
    );
}

/// A stream measure named no file, exactly as a stream page does.
#[test]
fn a_stream_measure_says_it_was_given_its_input() {
    let concept = only("wc -l");
    assert_eq!(
        concept,
        Concept::Measure {
            subjects: vec![],
            quantity: Quantity::Lines,
        }
    );
    assert_eq!(lower(&concept), "wc -l");
    assert_eq!(describe(&concept), "Count the lines of what it is given");
}

/// ⚠ **What measures and answers a different question stays in the queue.**
/// Bare `wc` is a TABLE (the POSIX triple), `-lc` likewise, `-L` a length;
/// `du` and `stat` are numbers about the FILE rather than its contents. None
/// made the census, so each queues for it rather than earning a name.
#[test]
fn a_measure_with_another_product_stays_queued() {
    for script in [
        "wc src/a.ts",
        "wc -lc src/a.ts",
        "wc -L src/a.ts",
        "wc --lines src/a.ts",
        "du -sh src/geo",
        "stat -c %y src/a.ts",
    ] {
        let found: Vec<Why> = steps(script)
            .iter()
            .filter_map(|step| lift(step).err())
            .collect();
        assert!(
            found.contains(&Why::NoLens),
            "`{script}` should queue, got {found:?}"
        );
    }
}

/// The shared guards, holding for the ninth lens as for the others: a redirect
/// is a subject the argv never spells, and `xargs wc -l` counts files a pipe
/// supplied — which is NOT the stream `wc -l` alone reads.
#[test]
fn a_measure_that_redirects_or_is_fed_by_xargs_does_not_lift() {
    for script in [
        "wc -l src/a.ts > count.txt",
        "wc -l < src/a.ts",
        "xargs wc -l",
    ] {
        let lifted: Vec<Concept> = steps(script)
            .iter()
            .filter_map(|step| lift(step).ok())
            .collect();
        assert!(lifted.is_empty(), "`{script}` lifted to {lifted:?}");
    }
}

/// The card sentence. A recursive listing says so, because "the entries of" and
/// "everything under" are different amounts of machine to touch.
#[test]
fn a_listing_describes_the_locus_and_its_reach() {
    assert_eq!(
        describe(&only("ls src/geo")),
        format!("List the entries of {CWD}/src/geo")
    );
    assert_eq!(
        describe(&only("ls -Ra src/geo")),
        format!("List everything under {CWD}/src/geo, hidden ones included")
    );
}

/// ⚠ **Acceptance test 1 for the fifth lens: `--oneline` is DECORATION.**
/// 94% of `git log` rows carry it (measured 2026-09-10 over 23,160 steps), and
/// it changes how a commit prints, never which commits appear — so it
/// normalises away exactly as `grep -n` does.
#[test]
fn a_git_log_decoration_flag_is_not_part_of_the_concept() {
    let bare = only("git log -3");
    let oneline = only("git log --oneline -3");
    let decorated = only("git log --oneline --no-pager --abbrev-commit -3");

    assert_eq!(bare, oneline);
    assert_eq!(bare, decorated);
    assert_eq!(
        bare,
        Concept::History {
            count: Some(3),
            from: None,
            paths: vec![],
        }
    );
}

/// ⚠ **The three spellings of a count are one count**, and the absence of one is
/// NOT a count. git's own default is unbounded, so inventing a number here would
/// be the fabrication the layer refuses — unlike `head`, whose ten is POSIX and
/// documented.
#[test]
fn a_git_log_count_is_read_in_every_spelling_and_never_invented() {
    for script in ["git log -3", "git log -n 3", "git log --max-count=3"] {
        let Concept::History { count, .. } = only(script) else {
            panic!("not a history: {script}");
        };
        assert_eq!(count, Some(3), "{script}");
    }
    let Concept::History { count, .. } = only("git log --oneline") else {
        panic!("not a history");
    };
    assert_eq!(
        count, None,
        "no count in the text means no count in the concept"
    );
}

/// **Gate 1 over every history shape**, including the revision and the paths
/// after `--` that the author declared to be paths.
#[test]
fn every_history_shape_survives_the_round_trip() {
    for script in [
        "git log -3",
        "git log --oneline",
        "git log --oneline -1 5710b66",
        "git log --oneline -3 -- src/a.ts",
        "git log -- src/a.ts src/b.ts",
        "git -C /tmp log --oneline -2",
    ] {
        let concept = only(script);
        let text = lower(&concept);
        assert_eq!(only(&text), concept, "lowered `{script}` to `{text}`");
    }
}

/// ⚠ **`git -C dir log` names a LOCATION, not a subject.** A repository is
/// context the way a working directory is, so the concept carries no repo field
/// and two logs of the same shape in different repos are the same concept —
/// which is what recurrence detection needs.
#[test]
fn a_repository_is_context_and_not_a_subject() {
    assert_eq!(
        only("git -C /tmp log --oneline -2"),
        only("git log --oneline -2")
    );
}

/// ⚠ **What selects different COMMITS, or hands back a different PRODUCT,
/// refuses by name.** Each is sized by the census and each would otherwise have
/// the concept name commits the command never showed.
#[test]
fn a_git_log_that_selects_or_formats_differently_refuses_by_name() {
    for (script, why) in [
        ("git log --all --oneline", Why::OtherSelection),
        ("git log --oneline --since=2026-09-03", Why::OtherSelection),
        ("git log --oneline --grep=fix", Why::OtherSelection),
        ("git log -S needle --oneline", Why::OtherSelection),
        (
            "git log --oneline --follow -- src/a.ts",
            Why::OtherSelection,
        ),
        ("git log -1 --format=%ai", Why::Formatted),
        ("git log --oneline -p", Why::Formatted),
        ("git log --oneline --stat", Why::Formatted),
        // Two revisions are a range spelled as two words, which this cannot say.
        ("git log --oneline HEAD~4 HEAD", Why::OtherSelection),
    ] {
        let found: Vec<Why> = steps(script)
            .iter()
            .filter_map(|step| lift(step).err())
            .collect();
        assert!(
            found.contains(&why),
            "`{script}` should refuse {why:?}, got {found:?}"
        );
    }
}

/// ⚠ **Every git subcommand is its OWN act and none may answer for another.**
/// `status`, `commit` and `add` have concepts of their own now; `push`, `diff`
/// and `show` stay in the queue where the census ranks them. Claiming any of
/// them as a `History` would be the flattening the vocabulary exists to avoid.
#[test]
fn each_git_subcommand_lifts_to_its_own_concept_or_none() {
    assert!(matches!(only("git status --short"), Concept::Status { .. }));
    assert!(matches!(only("git commit -m x"), Concept::Commit { .. }));
    assert!(matches!(only("git add src/a.ts"), Concept::Stage { .. }));
    assert!(matches!(only("git log -3"), Concept::History { .. }));
    for script in ["git push", "git diff --stat", "git show HEAD"] {
        let lifted: Vec<Concept> = steps(script)
            .iter()
            .filter_map(|step| lift(step).ok())
            .collect();
        assert!(lifted.is_empty(), "`{script}` lifted to {lifted:?}");
    }
}

/// The card sentence, and the one place the missing count must not read as one.
#[test]
fn a_history_describes_how_many_commits_and_from_where() {
    assert_eq!(describe(&only("git log -1")), "Show the last commit");
    assert_eq!(
        describe(&only("git log --oneline -3")),
        "Show the last 3 commits"
    );
    assert_eq!(
        describe(&only("git log --oneline")),
        "Show the commit history"
    );
    assert_eq!(
        describe(&only("git log --oneline -1 5710b66")),
        "Show the last commit from 5710b66"
    );
    assert_eq!(
        describe(&only("git log -3 -- notes.md")),
        format!("Show the last 3 commits touching {CWD}/notes.md")
    );
}

/// **Gate 1 over the three working-tree concepts.** `--short` and `-q` are
/// spellings; `-A`, `--amend` and `--no-verify` are not.
#[test]
fn every_git_working_tree_shape_survives_the_round_trip() {
    for script in [
        "git status",
        "git status --short",
        "git status --porcelain -b",
        "git status -- src/a.ts",
        "git add src/a.ts",
        "git add -A src/a.ts src/b.ts",
        "git commit -m 'fix the thing'",
        "git commit -q -m 'fix the thing'",
        "git commit --amend --no-edit -m x",
        "git commit --no-verify -m x",
        "git commit -F -",
    ] {
        let concept = only(script);
        let text = lower(&concept);
        assert_eq!(only(&text), concept, "lowered `{script}` to `{text}`");
    }
}

/// ⚠ **A message from a FILE is a hole, not an empty message.** `git commit -F -`
/// reads stdin; the message is real and is not in this text, exactly as
/// `sed -i -f fix.sed` has a substitution that is not. 2,972 of 10,758 commit
/// steps take this route, so it is the ordinary case and not an edge.
///
/// ⚠ And a hole must not lower to `-m ''`, which would invent an EMPTY message —
/// a different commit. It lowers to `-F -`, which reads back as the same hole.
#[test]
fn a_commit_message_that_is_not_in_the_text_is_a_hole() {
    let held = only("git commit -F -");
    assert_eq!(
        held,
        Concept::Commit {
            message: None,
            amend: false,
            no_verify: false
        }
    );
    assert_ne!(
        held,
        only("git commit -m ''"),
        "a hole and an empty message are different commits"
    );
    assert_eq!(lower(&held), "git commit -F -");
}

/// ⚠ **What stages differently, commits differently, or reports a different SET
/// refuses by name.** `-n`/`--dry-run` stages NOTHING, `-p` is interactive,
/// `commit -a` stages and commits in one act, `status --cached` reports only
/// what is staged. Each would have a lowered concept do something the command
/// did not.
#[test]
fn a_git_working_tree_command_that_does_something_else_refuses_by_name() {
    for (script, why) in [
        ("git add -n src/a.ts", Why::OtherSelection),
        ("git add -p src/a.ts", Why::OtherSelection),
        ("git add -u src/a.ts", Why::OtherSelection),
        ("git commit -a -m x", Why::OtherSelection),
        ("git commit --fixup HEAD", Why::OtherSelection),
        ("git status --cached", Why::OtherSelection),
        // `git add -A` alone stages the whole repository — a real subject the
        // text never wrote.
        ("git add -A", Why::ImplicitLocus),
    ] {
        let found: Vec<Why> = steps(script)
            .iter()
            .filter_map(|step| lift(step).err())
            .collect();
        assert!(
            found.contains(&why),
            "`{script}` should refuse {why:?}, got {found:?}"
        );
    }
}

/// ⚠ **A commit with no `-m` and no `-F` does not lift at all.** The message was
/// typed into an editor, and nothing in the text or the transcript records what
/// it said — so the concept would have to invent the one field that matters.
#[test]
fn a_commit_whose_message_was_typed_into_an_editor_does_not_lift() {
    let lifted: Vec<Concept> = steps("git commit")
        .iter()
        .filter_map(|step| lift(step).ok())
        .collect();
    assert!(lifted.is_empty(), "lifted to {lifted:?}");
}

/// The card sentences. Staging must never read as a write, and skipping the
/// gate must be said outright — both are what approval is FOR.
#[test]
fn the_git_working_tree_cards_say_what_approval_needs() {
    assert_eq!(
        describe(&only("git status --short")),
        "Show what the working tree has that the last commit does not"
    );
    assert_eq!(
        describe(&only("git add -A notes.md")),
        format!("Stage {CWD}/notes.md, deletions included")
    );
    assert_eq!(
        describe(&only("git commit -m 'fix the thing'")),
        "Commit the staged changes saying \"fix the thing\""
    );
    assert_eq!(
        describe(&only("git commit --no-verify -m x")),
        "Commit the staged changes saying \"x\" — SKIPPING the pre-commit gate"
    );
    assert_eq!(
        describe(&only("git commit -F -")),
        "Commit the staged changes with a message this command does not carry"
    );
}
