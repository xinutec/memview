//! Does the second gate actually compare anything?
//!
//! ⚠ **A gate that cannot fail is not a gate.** Over the corpus this one is
//! green on every command, and that is not by itself evidence: a `judge` that
//! returned `Agrees` unconditionally would look exactly the same. So each
//! verdict it can reach is provoked here on purpose.
//!
//! ⚠ **It is load-bearing where bash's printer NORMALISES**, and nowhere else.
//! Over plain words bash prints back what it was given, so there is nothing it
//! can see that the round-trip law cannot. Where it collapses two spellings into
//! one — a heredoc delimiter's quoting, a descriptor on `1>`, a word split at a
//! line continuation — it has caught real defects, and the last two are written
//! up at the nodes they corrected.

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
        // Heredocs: the delimiter's quoting is normalised to one spelling, the
        // body is not, and `<<-` keeps its dash over a body already stripped.
        "cat <<EOF\nbody\nEOF",
        "cat <<'EOF'\n$x stays\nEOF",
        "cat <<\"EOF\"\n$x stays\nEOF",
        "cat <<-EOF\n\tindented\n\tEOF",
        "cat <<A <<B\none\nA\ntwo\nB",
        "cat <<EOF | wc -l\nbody\nEOF",
        "cat <<EOF > out\nbody\nEOF",
        "cat 3<<EOF\nbody\nEOF",
        // An unquoted body's line continuation is resolved by bash at parse
        // time, so a printer that kept it would be printing a different string.
        "cat <<EOF\na\\\nb\nEOF",
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
    let verdicts = compare(&["cat <<< word".to_string()]).expect("the oracle should run");
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

// ---- gate 3: is our own print shell at all? ----

/// Print a command the way the printer does, so the fixtures below read as the
/// texts a user would see rather than as trees.
fn printed(text: &str) -> String {
    reader::syntax::print(&parse(text).expect("the fixture must parse"))
}

#[test]
fn the_printed_form_is_valid_shell() {
    // ⚠ The shapes where the printer chooses a SEPARATOR, which is where it can
    // emit text bash refuses. `a & ; b` is a syntax error where `a & b` is not,
    // and every compound below ends its body with one.
    let texts: Vec<String> = [
        "echo a",
        "for f in a; do b & done",
        "while a; do b & done",
        "until a; do b & done",
        "if a; then b & fi",
        "if a; then b; else c & fi",
        "for f in a; do for g in b; do c & done; done",
        "if a; then for f in x; do y & done; fi",
        "cat <<EOF\nbody\nEOF",
        "for f in a; do cat <<EOF\nx\nEOF\ndone",
        "echo ${x:-a b}",
        "echo ${x/a/b}",
        "echo ${a[0]}",
        "echo $(a | b)",
    ]
    .iter()
    .map(|text| printed(text))
    .collect();
    let verdicts = bash_oracle::validity(&texts).expect("bash -n runs");
    for (text, verdict) in texts.iter().zip(&verdicts) {
        assert_eq!(
            *verdict,
            bash_oracle::Validity::Parses,
            "bash refuses our print of {text:?}: {verdict:?}"
        );
    }
}

#[test]
fn the_gate_catches_a_print_bash_would_refuse() {
    // ⚠ **A gate that cannot fail is not a gate.** This is the exact text the
    // printer used to emit for a loop body ending in `&`, and it is why this
    // gate exists: gate 1 re-reads it happily with our own parser, and gate 2
    // never sees our print at all.
    let verdicts =
        bash_oracle::validity(&["for f in a; do b & ; done".to_string()]).expect("bash -n runs");
    assert!(
        matches!(verdicts[0], bash_oracle::Validity::Refused(_)),
        "bash should refuse `do b & ; done`: {verdicts:?}"
    );
}

#[test]
fn one_bad_print_in_a_batch_does_not_condemn_its_neighbours() {
    // The batch is one bash process while everything parses; a failure re-asks
    // each member alone. Without that, one bad text would mark the whole batch.
    let texts = vec![
        "echo before".to_string(),
        "for f in a; do b & ; done".to_string(),
        "echo after".to_string(),
    ];
    let verdicts = bash_oracle::validity(&texts).expect("bash -n runs");
    assert_eq!(verdicts[0], bash_oracle::Validity::Parses);
    assert!(matches!(verdicts[1], bash_oracle::Validity::Refused(_)));
    assert_eq!(verdicts[2], bash_oracle::Validity::Parses);
}
