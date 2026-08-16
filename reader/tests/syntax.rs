//! The tree's laws, stated as tests.
//!
//! The corpus is the coverage suite — `syntax-report` runs the round-trip law
//! over every distinct command there is. What is written by hand here is the
//! opposite: the handful of cases where a *wrong* tree would still satisfy the
//! law, so no corpus run could ever object. Quoting collapse, the glob/literal
//! split and the reserved words are all of that kind.

use reader::syntax::ast::{Glob, Item, Segment, SegmentKind, Span, Word};
use reader::syntax::{Outcome, Reason, check, parse, print};

fn tree(text: &str) -> reader::syntax::Script {
    parse(text).unwrap_or_else(|refusal| panic!("{text:?} was refused: {:?}", refusal.reason))
}

fn refusal(text: &str) -> Reason {
    parse(text)
        .err()
        .unwrap_or_else(|| panic!("{text:?} parsed, and should not have"))
        .reason
}

fn words(text: &str) -> Vec<Word> {
    match &tree(text).items[..] {
        [Item::Command(command)] => command.words.clone(),
        other => panic!("{text:?} is not one command: {other:?}"),
    }
}

// ---- spans are carried and never compared ----

#[test]
fn two_spans_are_equal_whatever_they_hold() {
    // The property every node's derived `PartialEq` rests on. Asserted directly
    // so that replacing it with a real comparison fails here rather than
    // silently making the round-trip law unsatisfiable.
    assert_eq!(Span::new(0, 1), Span::new(400, 9000));
}

#[test]
fn layout_changes_the_spans_and_not_the_tree() {
    assert_eq!(tree("echo a"), tree("echo    a"));
    assert_eq!(tree("a; b"), tree("a\nb"));
    assert_ne!(
        tree("echo a").items[0].clone(),
        tree("echo b").items[0].clone()
    );
}

#[test]
fn a_span_still_points_at_the_text() {
    let source = "echo hello";
    let word = &words(source)[1];
    assert_eq!(word.span.of(source), "hello");
}

// ---- quoting collapses, except where it means something ----

#[test]
fn the_three_spellings_of_a_word_are_one_word() {
    assert_eq!(words("echo a"), words("echo 'a'"));
    assert_eq!(words("echo a"), words(r#"echo "a""#));
    assert_eq!(words("echo abc"), words(r#"echo a"b"c"#));
}

#[test]
fn a_glob_is_not_the_character_that_spells_it() {
    // ⚠ The case the round-trip law cannot catch on its own: absorb `*` into a
    // literal and the text still prints and re-reads identically. It names a
    // different set of files, and nothing downstream could tell.
    let globbed = &words("ls a*b")[1];
    let quoted = &words("ls 'a*b'")[1];
    assert_ne!(globbed, quoted);
    assert_eq!(
        globbed.segments[1],
        Segment {
            kind: SegmentKind::Glob(Glob::Any),
            span: Span::new(0, 0),
        }
    );
    assert_eq!(quoted.as_literal().as_deref(), Some("a*b"));
    assert_eq!(globbed.as_literal(), None);
}

// ---- reserved words are grammar, and quoting decides which ----

#[test]
fn a_reserved_word_is_refused_and_a_quoted_one_is_a_command() {
    assert_eq!(refusal("time ./x.sh"), Reason::ReservedWord);
    assert_eq!(refusal("for f in a; do echo; done"), Reason::ReservedWord);
    // `'time' ./x.sh` runs /usr/bin/time — a program, and an ordinary word.
    assert_eq!(
        words("'time' ./x.sh")[0].as_literal().as_deref(),
        Some("time")
    );
}

#[test]
fn the_printer_quotes_a_word_that_would_read_back_as_grammar() {
    // Without this the tree for `'time' ./x.sh` prints as `time ./x.sh`, which
    // is a different program — and the law would catch it only because the
    // reprint is refused. Both halves are asserted.
    assert_eq!(print(&tree("'time' ./x.sh")), "'time' ./x.sh");
    assert!(check("'time' ./x.sh").holds());
}

#[test]
fn an_assignment_prefix_is_grammar_and_an_argument_is_not() {
    assert_eq!(refusal("FOO=bar cmd"), Reason::Assignment);
    assert_eq!(words("echo a=b")[1].as_literal().as_deref(), Some("a=b"));
    // The printer has to keep the difference when it writes one back.
    assert_eq!(print(&tree("'FOO=bar'")), "'FOO=bar'");
    assert!(check("'FOO=bar'").holds());
}

// ---- what is refused, and what is not ----

#[test]
fn every_construct_the_tree_does_not_model_is_named() {
    assert_eq!(refusal("a && b"), Reason::AndOr);
    assert_eq!(refusal("a || b"), Reason::AndOr);
    assert_eq!(refusal("a | b"), Reason::Pipe);
    assert_eq!(refusal("a &"), Reason::Background);
    assert_eq!(refusal("echo > out"), Reason::Redirection);
    assert_eq!(refusal("(cd x)"), Reason::Grouping);
    assert_eq!(refusal("echo $x"), Reason::Expansion);
    assert_eq!(refusal("echo `ls`"), Reason::Expansion);
    assert_eq!(refusal(r#"echo "$x""#), Reason::Expansion);
    assert_eq!(refusal("cd ~/Code"), Reason::Tilde);
    assert_eq!(refusal("~/bin/tool"), Reason::Tilde);
    // Bash agrees this one is unterminated: an apostrophe opens a quote.
    assert_eq!(refusal("echo it's"), Reason::UnterminatedQuote);
    assert_eq!(refusal("ls 'unclosed"), Reason::UnterminatedQuote);
}

#[test]
fn a_bracket_is_a_builtin_until_it_closes() {
    // `[ -f x ]` is the commonest conditional in the corpus and holds no
    // bracket expression: refusing every `[` would throw it away for a
    // construct that is not there.
    assert_eq!(words("[ -f x ]").len(), 4);
    assert_eq!(refusal("ls a[0-9]b"), Reason::BracketExpression);
}

// ---- comments are nodes ----

#[test]
fn a_comment_is_kept_byte_exact_and_put_back() {
    let script = tree("# a note about /home/example\necho a");
    match &script.items[0] {
        Item::Comment(comment) => assert_eq!(comment.text, " a note about /home/example"),
        other => panic!("expected a comment, got {other:?}"),
    }
    assert!(check("# a note about /home/example\necho a").holds());
    assert!(check("echo a # trailing").holds());
}

#[test]
fn a_hash_inside_a_word_is_not_a_comment() {
    assert_eq!(
        words("sed s#a#b#g")[1].as_literal().as_deref(),
        Some("s#a#b#g")
    );
}

// ---- the round-trip law ----

#[test]
fn the_law_holds_on_the_shapes_that_stress_the_printer() {
    for text in [
        "echo a",
        "echo 'a b'",
        "ls a*b",
        "ls 'a*b'",
        "echo ''",
        r#"echo "a'b""#,
        "echo a\\ b",
        "printf %s\\n x",
        "[ -f x ]",
        "cd /home/example/code",
        "# just a comment",
        "a; b; c",
        "echo -- --flag=value",
        "echo 'multi\nline'",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {:?}",
            check(text).label()
        );
    }
}

#[test]
fn the_law_reports_a_refusal_apart_from_a_failure() {
    // A refusal is the work queue, not a defect, and the two must never be
    // added together — a parser that refused everything would otherwise score
    // a perfect law.
    assert!(matches!(check("a | b"), Outcome::Refused(_)));
    assert!(!check("a | b").holds());
}
