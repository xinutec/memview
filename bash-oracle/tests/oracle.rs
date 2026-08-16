//! Does the second gate actually compare anything?
//!
//! ⚠ **A gate that cannot fail is not a gate.** Over the corpus this one is
//! green on every command, and that is not by itself evidence: a `judge` that
//! returned `Agrees` unconditionally would look exactly the same. So each
//! verdict it can reach is provoked here on purpose.
//!
//! ⚠ **It is not yet load-bearing, and that is expected.** The gate earns its
//! place where bash's printer *normalises* — desugaring `|&`, reordering
//! `! time`, laying a compound out with `do` on its own line — and the grammar
//! refuses all of those today. Over simple commands bash prints words back
//! verbatim, so there is nothing it can see that the round-trip law cannot. It
//! is wired now because adding it later would mean re-auditing everything
//! accepted without it.

use bash_oracle::{Verdict, compare};
use reader::syntax::ast::{Command, Pipeline, Script, Segment, SegmentKind, Span, Word};
use reader::syntax::parse;

fn nowhere() -> Span {
    Span::new(0, 0)
}

/// A pipeline of exactly one command, with no grammar on it.
fn one(command: Command) -> Pipeline {
    Pipeline {
        time: None,
        negated: false,
        commands: vec![command],
        span: nowhere(),
    }
}

fn literal(text: &str) -> Segment {
    Segment {
        kind: SegmentKind::Literal(text.to_string()),
        span: nowhere(),
    }
}

#[test]
fn bash_reads_our_printed_form_the_way_we_do() {
    let scripts: Vec<Script> = [
        "echo a",
        "echo 'a b'",
        "ls a*b",
        "ls 'a*b'",
        "[ -f x ]",
        "cd /home/example/code",
        "a; b; c",
        "echo -- --flag=value",
        // The two words that are only words because they are quoted.
        "'time' ./x.sh",
        "'FOO=bar'",
        // The pipeline shapes: bash reorders the prefixes and we must agree.
        "a | b | c",
        "time a | b",
        "time -p ls",
        "! grep -q x f",
        "! time a | b",
        "a | time b",
    ]
    .iter()
    .map(|text| parse(text).expect("fixture should parse"))
    .collect();

    let verdicts = compare(&scripts).expect("the oracle should run");
    for (script, verdict) in scripts.iter().zip(&verdicts) {
        assert!(
            verdict.agrees(),
            "bash disagreed about {script:?}: {verdict:?}"
        );
    }
}

#[test]
fn a_tree_bash_cannot_read_is_reported_not_passed() {
    // A command with no words prints an empty line, and a function with an
    // empty body is a syntax error. The parser never builds one — this is
    // constructed by hand precisely because it has to come from somewhere.
    let empty = Script {
        items: vec![reader::syntax::Item::Pipeline(one(Command {
            words: vec![],
            span: nowhere(),
        }))],
        span: nowhere(),
    };
    assert_eq!(compare(&[empty]).unwrap()[0], Verdict::BashRefused);
}

#[test]
fn a_tree_that_is_not_in_normal_form_is_caught() {
    // `Word { segments: [Literal("a"), Literal("b")] }` prints as `ab`, which
    // reads back as ONE segment. The trees differ, and the gate has to say so —
    // this is the shape of every real disagreement it will ever report.
    let unmerged = Script {
        items: vec![reader::syntax::Item::Pipeline(one(Command {
            words: vec![Word {
                segments: vec![literal("a"), literal("b")],
                span: nowhere(),
            }],
            span: nowhere(),
        }))],
        span: nowhere(),
    };
    assert!(
        matches!(compare(&[unmerged]).unwrap()[0], Verdict::Differs { .. }),
        "the gate passed a tree it should have rejected"
    );
}

#[test]
fn a_refusal_in_a_batch_does_not_swallow_its_neighbours() {
    // Bash aborts a script at the first syntax error, so one bad command
    // truncates the stream and every later one would be misattributed to the
    // wrong input. The fallback re-runs the batch one at a time; without it
    // this test reports `BashRefused` for the good commands too.
    let broken = Script {
        items: vec![reader::syntax::Item::Pipeline(one(Command {
            words: vec![],
            span: nowhere(),
        }))],
        span: nowhere(),
    };
    let good = parse("echo after").expect("fixture should parse");
    let verdicts = compare(&[broken, good]).expect("the oracle should run");
    assert_eq!(verdicts[0], Verdict::BashRefused);
    assert!(
        verdicts[1].agrees(),
        "the good command was lost: {:?}",
        verdicts[1]
    );
}
