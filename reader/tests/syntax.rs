//! The tree's laws, stated as tests.
//!
//! The corpus is the coverage suite — `syntax-report` runs the round-trip law
//! over every distinct command there is. What is written by hand here is the
//! opposite: the handful of cases where a *wrong* tree would still satisfy the
//! law, so no corpus run could ever object. Quoting collapse, the glob/literal
//! split and the reserved words are all of that kind.

use reader::syntax::ast::{
    Connector, Glob, Item, RedirectOp, RedirectTarget, Segment, SegmentKind, Span, Tilde, Timed,
    Word,
};
use reader::syntax::{Outcome, Reason, check, parse, print, survey};
use std::collections::BTreeSet;

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
        [Item::List(list)] if list.rest.is_empty() => match &list.first.commands[..] {
            [command] => command.words.clone(),
            other => panic!("{text:?} is not one command: {other:?}"),
        },
        other => panic!("{text:?} is not one plain pipeline: {other:?}"),
    }
}

fn pipeline(text: &str) -> reader::syntax::Pipeline {
    let list = list(text);
    assert!(
        list.rest.is_empty(),
        "{text:?} is an and-or list, not a pipeline"
    );
    list.first
}

fn list(text: &str) -> reader::syntax::AndOr {
    match &tree(text).items[..] {
        [Item::List(list)] => list.clone(),
        other => panic!("{text:?} is not one list: {other:?}"),
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
    assert_eq!(refusal("for f in a; do echo; done"), Reason::ReservedWord);
    // `time` is no longer refused — the pipeline models it. What still matters
    // is that the QUOTED one stays a program: `'time' ./x.sh` runs
    // /usr/bin/time, so its tree holds a word, not a flag.
    assert_eq!(pipeline("time ./x.sh").time, Some(Timed::Plain));
    assert_eq!(pipeline("'time' ./x.sh").time, None);
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
    assert_eq!(refusal("(cd x)"), Reason::Grouping);
    assert_eq!(refusal("echo $x"), Reason::Expansion);
    assert_eq!(refusal("echo `ls`"), Reason::Expansion);
    assert_eq!(refusal(r#"echo "$x""#), Reason::Expansion);
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
    assert!(matches!(check("cat <<EOF\nx\nEOF"), Outcome::Refused(_)));
    assert!(!check("cat <<EOF\nx\nEOF").holds());
}

// ---- the survey: which constructs a command needs, not which stopped us ----

#[test]
fn the_survey_returns_every_blocking_construct_not_the_first() {
    // The whole reason it exists: `parse` stops at the pipe and never sees the
    // redirection, so the refusal ranking under-counts whatever sits rightmost.
    assert_eq!(refusal("cat <<E\nx\nE\n$y"), Reason::Heredoc);
    assert_eq!(
        survey("cat <<E\nx\nE\n$y"),
        BTreeSet::from([Reason::Heredoc, Reason::Expansion])
    );
}

#[test]
fn an_argument_is_not_a_command_name() {
    // ⚠ Regression. Preserving `at_command_start` across a word made every word
    // on the line look like a command head, so `BatchMode=yes` was reported as
    // an assignment prefix and `in` as a reserved word — 191 corpus commands
    // where the survey claimed a construct the parser had accepted.
    assert!(survey("ssh -o BatchMode=yes host").is_empty());
    assert!(survey("git add in do done").is_empty());
    assert_eq!(survey("FOO=bar cmd"), BTreeSet::from([Reason::Assignment]));
    assert_eq!(survey("for f in a"), BTreeSet::from([Reason::ReservedWord]));
}

#[test]
fn the_survey_looks_past_what_it_cannot_own() {
    // A substitution's interior belongs to the layer that gets one, and a
    // heredoc body is data. Both are reported as themselves and not descended.
    assert_eq!(
        survey("echo $(git log | head)"),
        BTreeSet::from([Reason::Expansion])
    );
    assert_eq!(
        survey("cat <<EOF\na | b && c\nEOF"),
        BTreeSet::from([Reason::Heredoc])
    );
}

#[test]
fn the_survey_is_empty_exactly_when_the_parser_accepts() {
    // The invariant the corpus report re-checks on every row. Stated here too,
    // because a drift found on 131k commands is a worse place to find it.
    for text in [
        "echo a",
        "ls a*b",
        "[ -f x ]",
        "# a comment",
        "a; b",
        "'time' ./x.sh",
        "cd ~/x",
        "echo $x",
        "x > y",
        "(a)",
    ] {
        assert_eq!(
            survey(text).is_empty(),
            parse(text).is_ok(),
            "survey and parser disagree about {text:?}: {:?}",
            survey(text)
        );
    }
}

// ---- the pipeline ----

#[test]
fn a_pipeline_holds_its_commands_and_its_two_flags() {
    let p = pipeline("a | b | c");
    assert_eq!(p.commands.len(), 3);
    assert_eq!(p.time, None);
    assert!(!p.negated);
}

#[test]
fn time_and_bang_are_fields_not_argv() {
    // The misparse the flat reader still carries: `time` at `argv[0]` cannot say
    // whether the whole pipeline or only its first command is timed.
    let p = pipeline("time a | b");
    assert_eq!(p.time, Some(Timed::Plain));
    assert_eq!(p.commands.len(), 2);
    assert_eq!(p.commands[0].words[0].as_literal().as_deref(), Some("a"));

    assert_eq!(pipeline("time -p ls").time, Some(Timed::Posix));
    assert!(pipeline("! grep -q x f").negated);
}

#[test]
fn either_order_of_the_prefixes_is_one_tree() {
    // ⚠ Bash accepts both and prints `time` first — `! time a | b` comes back
    // from `declare -f` as `time ! a | b`. Two texts, one tree, and the printer
    // picks bash's spelling.
    assert_eq!(pipeline("! time a | b"), pipeline("time ! a | b"));
    assert_eq!(print(&tree("! time a | b")), "time ! a | b");
}

#[test]
fn negation_is_a_toggle_because_bash_collapses_it() {
    // `declare -f` prints `! ! a` back as `a`.
    assert!(!pipeline("! ! a").negated);
    assert!(pipeline("! ! ! a").negated);
    assert_eq!(print(&tree("! ! a")), "a");
}

#[test]
fn a_keyword_is_only_a_keyword_at_the_head() {
    // Measured: `a | time b` is accepted and runs /usr/bin/time, while
    // `a | ! b` is a syntax error. So one is a word and the other is refused.
    let p = pipeline("a | time b");
    assert_eq!(p.time, None);
    assert_eq!(p.commands[1].words[0].as_literal().as_deref(), Some("time"));
    assert_eq!(refusal("a | ! b"), Reason::ReservedWord);
}

#[test]
fn timeout_is_not_time() {
    // Without a word-boundary check the keyword eats the first four characters
    // of the commonest wrapper in the corpus.
    let p = pipeline("timeout 5 ls");
    assert_eq!(p.time, None);
    assert_eq!(
        p.commands[0].words[0].as_literal().as_deref(),
        Some("timeout")
    );
}

#[test]
fn the_printer_keeps_a_quoted_time_a_command_and_a_piped_one_bare() {
    // At the head, printing `time` bare would turn this program into a keyword.
    assert_eq!(print(&tree("'time' ./x.sh")), "'time' ./x.sh");
    // After a pipe it is already a program name, so quoting would be noise.
    assert_eq!(print(&tree("a | time b")), "a | time b");
    assert!(check("'time' ./x.sh").holds());
    assert!(check("a | time b").holds());
}

#[test]
fn the_law_holds_across_the_pipeline_shapes() {
    for text in [
        "a | b",
        "a | b | c",
        "time a | b",
        "time -p a",
        "! a | b",
        "! time a | b",
        "ls -1 | wc -l",
        "a|b",
        "a |b| c",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
    }
}

#[test]
fn a_pipeline_prefix_does_not_start_the_command() {
    // ⚠ Regression, found by the corpus invariant on ONE command out of 131k:
    // `time PYTHONPATH=… python -m recall doctor`. Treating `time` as the
    // command name made the assignment after it look like an argument, so the
    // survey reported nothing where the parser refused.
    assert_eq!(
        survey("time PYTHONPATH=/x python -m recall"),
        BTreeSet::from([Reason::Assignment])
    );
    assert_eq!(
        refusal("time PYTHONPATH=/x python -m recall"),
        Reason::Assignment
    );
    // `-p` is a prefix only after `time`; alone it is an ordinary argument.
    assert_eq!(
        survey("time -p FOO=bar x"),
        BTreeSet::from([Reason::Assignment])
    );
}

// ---- and-or lists ----

#[test]
fn a_list_holds_its_connectors_in_order() {
    let l = list("a && b || c");
    assert_eq!(l.rest.len(), 2);
    assert_eq!(l.rest[0].connector, Connector::And);
    assert_eq!(l.rest[1].connector, Connector::Or);
    assert!(!l.background);
}

#[test]
fn background_belongs_to_the_list_not_its_last_pipeline() {
    // ⚠ `a && b &` backgrounds the WHOLE list — `declare -f` prints it back
    // that way. Hanging the flag on `b` would say something else.
    let l = list("a && b &");
    assert!(l.background);
    assert_eq!(l.rest.len(), 1);
    assert_eq!(print(&tree("a && b &")), "a && b &");
}

#[test]
fn a_semicolon_separates_lists_and_a_connector_does_not() {
    // Bash's own split: `a; b` prints on two lines, `a && b` on one.
    assert_eq!(tree("a; b").items.len(), 2);
    assert_eq!(tree("a && b").items.len(), 1);
    assert_eq!(tree("a & b").items.len(), 2);
    assert!(list("a &").background);
}

#[test]
fn a_newline_after_a_connector_continues_the_list() {
    // Same rule as a newline after `|`, and the same silent misparse if missed.
    assert_eq!(tree("a &&\nb"), tree("a && b"));
    assert_eq!(tree("a ||\n  b"), tree("a || b"));
}

#[test]
fn an_operator_with_nothing_after_it_is_refused() {
    assert_eq!(refusal("a &&"), Reason::EmptyOperand);
    assert_eq!(refusal("a |"), Reason::EmptyOperand);
    // ⚠ Bash accepts a comment here and DELETES it. Keeping comments byte-exact
    // and accepting this are incompatible, so it is refused rather than dropped.
    assert_eq!(refusal("a && # note\nb"), Reason::CommentInList);
}

#[test]
fn the_law_holds_across_the_list_shapes() {
    for text in [
        "a && b",
        "a || b",
        "a && b || c",
        "a | b && c | d",
        "a && b &",
        "a &",
        "time a && b",
        "! a || b",
        "a &&\nb",
        "a; b && c; d",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
    }
}

// ---- redirection ----

fn redirects(text: &str) -> Vec<reader::syntax::Redirect> {
    match &list(text).first.commands[..] {
        [command] => command.redirects.clone(),
        other => panic!("{text:?} is not one command: {other:?}"),
    }
}

#[test]
fn a_redirect_is_not_a_word_and_its_position_is_not_recorded() {
    // ⚠ Bash prints `> out cat f` back as `cat f > out`, so the position among
    // the words carries nothing. Both spellings must be one tree.
    assert_eq!(tree("> out cat f"), tree("cat f > out"));
    assert_eq!(words("cat f > out").len(), 2);
    assert_eq!(redirects("cat f > out").len(), 1);
}

#[test]
fn the_order_of_redirects_among_themselves_does_matter() {
    // `cat > out 2>&1` and `cat 2>&1 > out` send stderr to different places.
    assert_ne!(tree("cat > out 2>&1"), tree("cat 2>&1 > out"));
}

#[test]
fn every_form_reaches_its_own_node() {
    use RedirectOp::*;
    for (text, op) in [
        ("cat < in", Read),
        ("cat > out", Write),
        ("cat >> out", Append),
        ("cat <> rw", ReadWrite),
        ("cat >| out", Clobber),
        ("cat 2>&1", DupOut),
        ("cat 0<&3", DupIn),
        ("cat &> both", Both),
        ("cat &>> both", BothAppend),
        ("cat >& both", BothWord),
    ] {
        assert_eq!(redirects(text)[0].op, op, "{text:?}");
    }
    assert_eq!(redirects("cat 2>&-")[0].target, RedirectTarget::Close);
    assert_eq!(redirects("cat 2> err")[0].fd, Some(2));
}

#[test]
fn the_dup_forms_default_their_descriptor_and_the_others_do_not() {
    // ⚠ `>&2` comes back from `declare -f` as `1>&2`, so the two are one tree.
    assert_eq!(tree("cat >&2"), tree("cat 1>&2"));
    assert_eq!(redirects("cat >&2")[0].fd, Some(1));
    // `> out` never gains a descriptor, and bash never prints one.
    // `> out` means fd 1 whether it says so or not, and bash agrees by
    // printing `1> out` back as `> out`.
    assert_eq!(redirects("cat > out")[0].fd, Some(1));
    assert_eq!(tree("cat 1> out"), tree("cat > out"));
    assert_eq!(tree("cat 0< in"), tree("cat < in"));
    assert_eq!(redirects("cat &> both")[0].fd, None);
}

#[test]
fn a_descriptor_must_touch_its_operator() {
    // `cat 2>out` redirects fd 2; `cat 2 > out` passes 2 as an argument.
    assert_eq!(redirects("cat 2>out")[0].fd, Some(2));
    assert_eq!(redirects("cat 2 > out")[0].fd, Some(1));
    assert_eq!(words("cat 2 > out").len(), 2);
}

#[test]
fn what_is_still_refused_is_named() {
    assert_eq!(refusal("cat <<EOF\nx\nEOF"), Reason::Heredoc);
    assert_eq!(refusal("cat <<< word"), Reason::HereString);
    assert_eq!(refusal("diff <(a) <(b)"), Reason::ProcessSubstitution);
    assert_eq!(refusal("cat >"), Reason::EmptyOperand);
}

#[test]
fn the_law_holds_across_the_redirection_shapes() {
    for text in [
        "cat > out",
        "cat >> out",
        "cat < in",
        "cat 2> err",
        "cat 2>&1",
        "cat >&2",
        "cat &> both",
        "cat 2>&-",
        "cat >| out",
        "cat <> rw",
        "> out cat f",
        "cat f > out 2>&1",
        "a > x | b",
        "a && b > x",
        "cat > out &",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
    }
}

// ---- tilde ----

#[test]
fn a_tilde_expands_only_where_the_shell_expands_one() {
    // ⚠ The quoting rule that is semantic INSIDE a word: `~/x` is a home
    // directory, `"~/x"` is a filename that starts with a tilde. Absorbing the
    // first into a literal is the error the refusal discipline exists for, and
    // it is exactly what the parser did until the tilde got a node.
    assert_ne!(words("cd ~/x")[1], words("cd '~/x'")[1]);
    assert_eq!(words("cd '~/x'")[1].as_literal().as_deref(), Some("~/x"));
    assert_eq!(words("cd ~/x")[1].as_literal(), None);
    // Mid-word it is an ordinary character: only the head expands.
    assert_eq!(words("echo a~b")[1].as_literal().as_deref(), Some("a~b"));
}

#[test]
fn each_tilde_form_reaches_its_own_node() {
    let head = |text: &str| match &words(text)[1].segments[0].kind {
        SegmentKind::Tilde(tilde) => tilde.clone(),
        other => panic!("{text:?} is not a tilde: {other:?}"),
    };
    assert_eq!(head("cd ~"), Tilde::Home);
    assert_eq!(head("cd ~/Code"), Tilde::Home);
    assert_eq!(head("cd ~+"), Tilde::Pwd);
    assert_eq!(head("cd ~-"), Tilde::OldPwd);
    assert_eq!(head("cd ~pippijn/x"), Tilde::User("pippijn".into()));
    // A directory-stack entry is a different construct and stays refused.
    assert_eq!(refusal("cd ~+2"), Reason::Tilde);
}

#[test]
fn the_printer_quotes_a_literal_that_would_expand() {
    // A word whose text merely starts with `~` must come back as text.
    assert_eq!(print(&tree("cd '~/x'")), "cd '~/x'");
    assert_eq!(print(&tree("cd ~/x")), "cd ~/x");
    assert!(check("cd '~/x'").holds());
}

#[test]
fn the_law_holds_across_the_tilde_shapes() {
    for text in [
        "cd ~",
        "cd ~/Code",
        "cd ~+",
        "cd ~-",
        "cd ~user/x",
        "ls ~/*.log",
        "cd '~'",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
    }
}

#[test]
fn a_quote_may_not_touch_a_tilde_prefix() {
    // ⚠ Regression, 319 corpus commands. `~'/x'` is the literal `~/x` to bash,
    // so quoting the segment after a tilde changes what the word means. The
    // slash goes through bare and closes the prefix; quoting is safe after it.
    assert_eq!(
        print(&tree(r"cat ~/.config/Local\ State")),
        "cat ~/'.config/Local State'"
    );
    assert!(check(r"cat ~/.config/Local\ State").holds());
    // With no slash to close the prefix, backslashes are what put it back.
    assert_eq!(print(&tree(r"cd ~user\ x")), r"cd ~user\ x");
    assert!(check(r"cd ~user\ x").holds());
}

#[test]
fn a_backslash_newline_joins_a_word() {
    // ⚠ Regression found by the second gate on one corpus command. Bash removes
    // the continuation and keeps ONE word; ending the word here split a single
    // long argument into three, and both the law and the survey agreed with it.
    assert_eq!(words("perl -e \"a\"\\\n\"b\"").len(), 3);
    assert_eq!(
        words("perl -e \"a\"\\\n\"b\"")[2].as_literal().as_deref(),
        Some("ab")
    );
    assert_eq!(words("cmd one\\\ntwo").len(), 2);
    assert!(check("perl -e \"a\"\\\n\"b\"").holds());
}
