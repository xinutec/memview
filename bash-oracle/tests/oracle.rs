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
use reader::syntax::parse;

#[test]
fn bash_reads_our_printed_form_the_way_we_do() {
    let commands: Vec<String> = [
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
    .map(|text| {
        parse(text).expect("fixture should parse");
        text.to_string()
    })
    .collect();

    let verdicts = compare(&commands).expect("the oracle should run");
    for (command, verdict) in commands.iter().zip(&verdicts) {
        assert!(
            verdict.agrees(),
            "bash disagreed about {command:?}: {verdict:?}"
        );
    }
}

#[test]
fn the_gate_catches_a_misparse_of_the_original_text() {
    // ⚠ The reason bash is shown the corpus command and not our print of it.
    //
    // `a |⏎b` is ONE pipeline: bash's grammar is `pipeline '|' newline_list
    // pipeline`. Read as two, it printed as two lines and read back as two just
    // as wrongly, so the round-trip law held — and the earlier version of this
    // gate, fed that same printed form, agreed with the mistake. Fed the
    // original, bash prints `a | b` and the disagreement is visible.
    let commands = vec!["a |\nb".to_string(), "a | b".to_string()];
    let verdicts = compare(&commands).expect("the oracle should run");
    assert!(verdicts.iter().all(|v| v.agrees()), "{verdicts:?}");
    // Both spellings are one tree, which is what bash says too.
    assert_eq!(parse("a |\nb").unwrap(), parse("a | b").unwrap());
}

#[test]
fn a_command_we_cannot_read_is_reported_not_scored_as_agreement() {
    // The gate promises to run only on accepted commands. Handed one that is
    // not, it has to say so rather than quietly count a pass.
    let verdicts = compare(&["a > b".to_string()]).expect("the oracle should run");
    assert!(
        matches!(verdicts[0], Verdict::Unreadable(_)),
        "{verdicts:?}"
    );
}

#[test]
fn a_refusal_in_a_batch_does_not_swallow_its_neighbours() {
    // Bash aborts a script at the first syntax error, so one bad command
    // truncates the stream and every later one would be misattributed to the
    // wrong input. The fallback re-runs the batch one at a time.
    let commands = vec!["ls 'unterminated".to_string(), "echo after".to_string()];
    let verdicts = compare(&commands).expect("the oracle should run");
    assert!(!verdicts[0].agrees());
    assert!(
        verdicts[1].agrees(),
        "the good command was lost: {:?}",
        verdicts[1]
    );
}
