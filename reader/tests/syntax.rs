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
    simple(text).words.clone()
}

/// The nth command of a pipeline, as a simple command.
fn nth(pipeline: &reader::syntax::Pipeline, index: usize) -> reader::syntax::ast::Simple {
    match &pipeline.commands[index].kind {
        reader::syntax::ast::CommandKind::Simple(simple) => simple.clone(),
        other => panic!("command {index} is not simple: {other:?}"),
    }
}

fn simple(text: &str) -> reader::syntax::ast::Simple {
    match &tree(text).items[..] {
        [Item::List(list)] if list.rest.is_empty() => match &list.first.commands[..] {
            [command] => match &command.kind {
                reader::syntax::ast::CommandKind::Simple(simple) => simple.clone(),
                other => panic!("{text:?} is not a simple command: {other:?}"),
            },
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
    assert_eq!(refusal("case $x in a) b;; esac"), Reason::Case);
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
    assert_eq!(words("echo a=b")[1].as_literal().as_deref(), Some("a=b"));
    // The printer has to keep the difference when it writes one back.
    assert_eq!(print(&tree("'FOO=bar'")), "'FOO=bar'");
    assert!(check("'FOO=bar'").holds());
}

// ---- what is refused, and what is not ----

#[test]
fn every_construct_the_tree_does_not_model_is_named() {
    assert_eq!(refusal("(cd x)"), Reason::Grouping);
    assert_eq!(refusal("echo `ls`"), Reason::Backtick);
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
    assert!(matches!(check("cat <<< word"), Outcome::Refused(_)));
    assert!(!check("cat <<< word").holds());
}

// ---- the survey: which constructs a command needs, not which stopped us ----

#[test]
fn the_survey_returns_every_blocking_construct_not_the_first() {
    // The whole reason it exists: `parse` stops at the first thing it cannot
    // read and never sees the rest, so the refusal ranking under-counts
    // whatever sits rightmost.
    assert_eq!(refusal("ls a[0-9]b | grep `y`"), Reason::BracketExpression);
    assert_eq!(
        survey("ls a[0-9]b | grep `y`"),
        BTreeSet::from([Reason::BracketExpression, Reason::Backtick])
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
    assert!(survey("FOO=bar cmd").is_empty());
    // A keyword at the head of a command still is one. (`case` also trips the
    // survey's grouping scan on its `)`, which is over-reporting the invariant
    // allows — the parser refuses `case` first either way.)
    assert!(survey("case x in a) b;; esac").contains(&Reason::Case));
}

#[test]
fn the_survey_looks_past_what_it_cannot_own() {
    // A heredoc body is data, and a substitution's interior is a script the
    // parser reads — neither is a finding. What the survey still reports from
    // inside a substitution is only what the parser refuses there.
    assert!(survey("echo $(git log | head)").is_empty());
    assert!(survey("cat <<EOF\na | b && c\nEOF").is_empty());
    assert!(survey("echo `git log`").contains(&Reason::Backtick));
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
    assert_eq!(nth(&p, 0).words[0].as_literal().as_deref(), Some("a"));

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
    assert_eq!(nth(&p, 1).words[0].as_literal().as_deref(), Some("time"));
    assert_eq!(refusal("a | ! b"), Reason::MisplacedNegation);
}

#[test]
fn timeout_is_not_time() {
    // Without a word-boundary check the keyword eats the first four characters
    // of the commonest wrapper in the corpus.
    let p = pipeline("timeout 5 ls");
    assert_eq!(p.time, None);
    assert_eq!(nth(&p, 0).words[0].as_literal().as_deref(), Some("timeout"));
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
    // The binding itself is modelled now, so the property is asserted with a
    // construct that still is not: the word after `time` is the command NAME,
    // and a reserved word there is grammar.
    assert_eq!(
        survey("time PYTHONPATH=/x if"),
        BTreeSet::from([Reason::Conditional])
    );
    assert_eq!(refusal("time PYTHONPATH=/x if"), Reason::Conditional);
    // `-p` is a prefix only after `time`; alone it is an ordinary argument.
    assert_eq!(survey("time -p if"), BTreeSet::from([Reason::Conditional]));
    // And a binding after `time` is read, not refused.
    assert!(survey("time PYTHONPATH=/x python -m recall").is_empty());
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

// ---- heredocs ----

fn here(text: &str) -> reader::syntax::ast::Heredoc {
    match &redirects(text)[..] {
        [redirect] => match &redirect.target {
            RedirectTarget::Here(here) => here.clone(),
            other => panic!("{text:?} redirects to {other:?}, not a heredoc"),
        },
        other => panic!("{text:?} is not one redirection: {other:?}"),
    }
}

#[test]
fn every_quoted_spelling_of_a_delimiter_is_one_tree() {
    // ⚠ The distinction bash itself does not keep: `declare -f` prints all four
    // of these back as `<<'EOF'`. A tree that recorded the spelling would say
    // they differ, and the second gate — which compares bash's rendering of each
    // — could never object, because it is bash that collapsed them.
    let quoted = [
        "cat <<'EOF'\n$x\nEOF",
        "cat <<\"EOF\"\n$x\nEOF",
        "cat <<\\EOF\n$x\nEOF",
    ];
    for text in quoted {
        let here = here(text);
        assert_eq!(here.delimiter, "EOF", "delimiter of {text:?}");
        assert!(here.quoted, "{text:?} has a quoted delimiter");
        assert_eq!(here.body, "$x\n");
    }
    assert_eq!(here(quoted[0]), here(quoted[1]));
    assert_eq!(here(quoted[0]), here(quoted[2]));

    // And the unquoted form is a DIFFERENT tree, because the body expands.
    let plain = here("cat <<EOF\n$x\nEOF");
    assert!(!plain.quoted);
    assert_ne!(plain, here(quoted[0]));
}

#[test]
fn a_body_is_data_and_reaches_the_tree_whole() {
    // Nothing in a body is a construct: an unclosed quote, a `$(`, a reserved
    // word and a `|` are all just bytes, and scanning them as shell would invent
    // refusals nobody wrote.
    let here = here("cat <<'PY'\nif x | y then '\ndone $(\nPY");
    assert_eq!(here.body, "if x | y then '\ndone $(\n");
    assert!(survey("cat <<'PY'\nif x | y then '\ndone $(\nPY").is_empty());
}

#[test]
fn an_empty_body_is_not_a_missing_one() {
    assert_eq!(here("cat <<EOF\nEOF").body, "");
    assert!(check("cat <<EOF\nEOF").holds());
}

#[test]
fn a_dash_strips_leading_tabs_and_only_tabs() {
    // ⚠ Bash strips at PARSE time and prints the `-` back with an unindented
    // body, so the tree holds the stripped text and the operator both.
    assert_eq!(here("cat <<-EOF\n\tbody\n\tEOF").body, "body\n");
    assert_eq!(
        redirects("cat <<-EOF\n\tbody\n\tEOF")[0].op,
        RedirectOp::HereDash
    );
    assert_eq!(redirects("cat <<EOF\nbody\nEOF")[0].op, RedirectOp::Here);
    // Spaces are not tabs: they stay in the body, and a terminator behind one
    // does not terminate.
    assert_eq!(here("cat <<-EOF\n\t  spaced\n\tEOF").body, "  spaced\n");
    // A terminator behind a space is not one, so the body runs to the end.
    assert_eq!(here("cat <<-EOF\nbody\n\t EOF").body, "body\n EOF\n");
}

#[test]
fn a_terminator_is_matched_exactly() {
    // Bash does not trim, so `EOF ` is body text and the body runs to the end
    // of the input — which is where bash ends it too.
    assert_eq!(here("cat <<EOF\nbody\nEOF ").body, "body\nEOF \n");
    assert_eq!(here("cat <<EOF\nbody\n EOF").body, "body\n EOF\n");
    // Nor is a substring one.
    assert_eq!(here("cat <<EOF\nEOFEOF\nEOF").body, "EOFEOF\n");
}

#[test]
fn several_heredocs_on_one_line_take_their_bodies_in_order() {
    // ⚠ The reason the body-to-opener match is positional: two heredocs may
    // share a delimiter, so nothing in the text pairs them but order.
    let redirects = redirects("cat <<A <<A\none\nA\ntwo\nA");
    let bodies: Vec<String> = redirects
        .iter()
        .map(|redirect| match &redirect.target {
            RedirectTarget::Here(here) => here.body.clone(),
            other => panic!("not a heredoc: {other:?}"),
        })
        .collect();
    assert_eq!(bodies, ["one\n", "two\n"]);
    assert!(check("cat <<A <<A\none\nA\ntwo\nA").holds());
}

#[test]
fn the_body_starts_after_the_logical_line_not_the_next_newline() {
    // ⚠ Both measured in `reader/probes/heredoc.sh`, and both are why the body
    // is read where a line ending is CONSUMED rather than by scanning ahead for
    // the next `\n`.
    assert_eq!(here("cat <<EOF \\\nextra\nbody\nEOF").body, "body\n");
    assert_eq!(here("cat <<EOF \"q\nr\"\nbody\nEOF").body, "body\n");
    // A heredoc opened before a `|` or a `&&` still takes the line's end.
    let piped = list("cat <<EOF |\nbody\nEOF\nwc -l");
    match &piped.first.commands[0].redirects[0].target {
        RedirectTarget::Here(here) => assert_eq!(here.body, "body\n"),
        other => panic!("not a heredoc: {other:?}"),
    }
    assert_eq!(
        here("cat <<EOF && true\nbody\nEOF").body,
        "body\n",
        "the body follows the whole list, not the first command"
    );
}

#[test]
fn a_backslash_newline_joins_an_unquoted_body_and_not_a_quoted_one() {
    // ⚠ Bash resolves the continuation at parse time, so `quoted` decides what
    // the body STRING is and not only what will expand later.
    assert_eq!(here("cat <<EOF\na\\\nb\nEOF").body, "ab\n");
    assert_eq!(here("cat <<'EOF'\na\\\nb\nEOF").body, "a\\\nb\n");
    // And the join is not a text replacement: an escaped backslash protects the
    // newline behind it.
    assert_eq!(here("cat <<EOF\na\\\\\nb\nEOF").body, "a\\\\\nb\n");
    // A backslash before anything else keeps both characters.
    assert_eq!(here("cat <<EOF\n\\$lit\nEOF").body, "\\$lit\n");
}

#[test]
fn a_join_that_would_forge_a_terminator_is_refused() {
    // `EO\` + `F` becomes the line `EOF`, which the printer has no way to write
    // back without ending the heredoc early.
    assert_eq!(refusal("cat <<EOF\nEO\\\nF\nEOF"), Reason::EmptyOperand);
}

#[test]
fn a_heredoc_defaults_to_stdin_and_takes_a_descriptor() {
    assert_eq!(redirects("cat <<EOF\nx\nEOF")[0].fd, Some(0));
    assert_eq!(redirects("cat 3<<EOF\nx\nEOF")[0].fd, Some(3));
    assert!(check("cat 3<<EOF\nx\nEOF").holds());
}

#[test]
fn a_body_with_no_terminator_runs_to_the_end_of_the_input() {
    // ⚠ **Read, not refused.** Bash takes the rest of the input as the body and
    // warns; the corpus is shell history, so these are commands that really ran
    // and refusing them would drop real work. `declare -f` cannot render one —
    // the runaway body eats the wrapper's brace — so gate 2 excludes them the
    // way it excludes comments, and gate 1 covers them alone.
    assert_eq!(here("cat <<EOF").body, "");
    assert_eq!(here("cat <<EOF\nbody").body, "body\n");
    // The shape the corpus actually holds: two openers and one body, so the
    // second heredoc gets what is left, which is nothing.
    let bodies: Vec<String> = redirects("python3 - <<'EOF' || python3 - <<'EOF'\nprint(1)\nEOF")
        .iter()
        .map(|redirect| match &redirect.target {
            RedirectTarget::Here(here) => here.body.clone(),
            other => panic!("not a heredoc: {other:?}"),
        })
        .collect();
    assert_eq!(bodies, ["print(1)\n"]);
    // An empty delimiter, whose terminator is an empty line, is the same rule
    // seen from the other side.
    assert_eq!(here("cat <<''\nbody\n\n").body, "body\n");
    assert_eq!(refusal("cat <<"), Reason::EmptyOperand);
    assert_eq!(refusal("cat <<$x\nbody\n$x"), Reason::Parameter);
}

#[test]
fn the_survey_agrees_with_the_parser_about_heredocs() {
    for text in [
        "cat <<EOF\nbody\nEOF",
        "cat <<-'EOF'\n\tbody\n\tEOF",
        "cat <<A <<B\none\nA\ntwo\nB",
        "cat <<EOF",
        "cat <<EOF\nbody\nEOF ",
        "cat <<$x\nbody\n$x",
        "cat <<EOF\n$(danger)\nEOF",
    ] {
        let found = survey(text);
        match parse(text) {
            Ok(_) => assert!(
                found.is_empty(),
                "{text:?} parsed but the survey found {found:?}"
            ),
            Err(refusal) => assert!(
                found.contains(&refusal.reason),
                "{text:?} was refused for {:?}, which the survey missed: {found:?}",
                refusal.reason
            ),
        }
    }
}

#[test]
fn the_law_holds_across_the_heredoc_shapes() {
    for text in [
        "cat <<EOF\nbody\nEOF",
        "cat <<'EOF'\n$x stays\nEOF",
        "cat <<-EOF\n\tindented\n\tEOF",
        "cat <<EOF\nEOF",
        "cat <<EOF > out\nbody\nEOF",
        "cat > out <<EOF\nbody\nEOF",
        "cat <<EOF | wc -l\nbody\nEOF",
        "cat <<A <<B\none\nA\ntwo\nB",
        "cat <<EOF && true\nbody\nEOF",
        "cat <<EOF\nbody\nEOF\necho after",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
    }
}

// ---- what a `$` opens, and when it opens nothing ----

#[test]
fn each_dollar_form_is_named_apart() {
    // ⚠ A reason is a unit of work, and these are not one build: naming a
    // parameter is a leaf, `${x%%y}` is a small language, and `$(…)` is a whole
    // script this parser would have to recurse into.
    // The operator family is built now; what is left of this reason is the
    // substring, whose operands are arithmetic.
    assert_eq!(refusal("echo ${x:1:3}"), Reason::ParameterOperator);
    assert!(parse("echo ${x:-y}").is_ok());
    assert!(parse("echo ${#x}").is_ok());
    assert!(parse("echo ${x%%.*}").is_ok());
    assert_eq!(refusal("echo `date`"), Reason::Backtick);
    assert_eq!(refusal("echo $((1+2))"), Reason::Arithmetic);
    assert_eq!(refusal("echo $'\\x41'"), Reason::AnsiQuote);
    assert_eq!(refusal("echo $\"hello\""), Reason::LocaleQuote);
}

#[test]
fn a_dollar_that_opens_nothing_is_an_ordinary_character() {
    // ⚠ Measured, not assumed: bash parses all of these and prints them back
    // unchanged, so refusing them would drop commands over a character that
    // expands to itself.
    assert_eq!(words("echo $")[1].as_literal().as_deref(), Some("$"));
    assert_eq!(words("echo a$")[1].as_literal().as_deref(), Some("a$"));
    assert_eq!(words("echo $.")[1].as_literal().as_deref(), Some("$."));
    assert_eq!(words(r#"echo "5$""#)[1].as_literal().as_deref(), Some("5$"));
    for text in ["echo $", "echo a$", "echo $.", r#"echo "5$""#] {
        assert!(check(text).holds(), "the law failed on {text:?}");
        assert!(survey(text).is_empty(), "{text:?} should need nothing");
    }
    // Inside double quotes a following quote is an ordinary character, so the
    // two quoting forms are only reachable from unquoted text.
    assert_eq!(refusal("echo $'a'"), Reason::AnsiQuote);
    assert_eq!(
        words(r#"echo "$'a'""#)[1].as_literal().as_deref(),
        Some("$'a'")
    );
}

#[test]
fn a_binding_is_decided_by_the_name_not_by_the_word() {
    // ⚠ Regression, and a silent one: `FOO="bar" cmd` parsed as a command NAMED
    // `FOO=bar`, because the check ran on the finished word and any quote in it
    // turned the check off. Bash asks only whether the NAME was quoted — all
    // four measured — and a wrong tree here prints and re-reads as itself, so
    // neither gate could ever object.

    // A quote at the name makes it an ordinary, oddly-named command.
    assert_eq!(
        words("'FOO=bar' cmd")[0].as_literal().as_deref(),
        Some("FOO=bar")
    );
    assert_eq!(
        words(r#""FOO"=bar cmd"#)[0].as_literal().as_deref(),
        Some("FOO=bar")
    );
    // Only the first word is a binding; elsewhere it is an argument.
    assert_eq!(
        words("echo FOO=bar")[1].as_literal().as_deref(),
        Some("FOO=bar")
    );
    for text in ["FOO=bar cmd", r#"FOO="bar" cmd"#, "FOO+=bar cmd"] {
        assert!(survey(text).is_empty(), "{text:?} should need nothing");
        assert!(check(text).holds(), "the law failed on {text:?}");
    }
    assert!(survey("'FOO=bar' cmd").is_empty());
}

// ---- parameters ----

fn segments(text: &str, index: usize) -> Vec<Segment> {
    words(text)[index].segments.clone()
}

fn parameter(text: &str) -> reader::syntax::ast::Parameter {
    match &segments(text, 1)[..] {
        [
            Segment {
                kind: SegmentKind::Parameter(p),
                ..
            },
        ] => p.clone(),
        other => panic!("{text:?} is not one parameter: {other:?}"),
    }
}

#[test]
fn quoting_a_parameter_is_semantic_and_quoting_a_literal_is_not() {
    // ⚠ The distinction the tree MUST keep, and the reason `quoted` is a field
    // rather than a print-time choice: an unquoted expansion is split into words
    // and then globbed, a quoted one is one word whatever it holds. `'a'`, `"a"`
    // and `a` collapse; `$x` and `"$x"` do not.
    assert!(!parameter("echo $x").quoted);
    assert!(parameter(r#"echo "$x""#).quoted);
    assert_ne!(words("echo $x"), words(r#"echo "$x""#));
    // ...while the literal spellings still collapse, as before.
    assert_eq!(words("echo a"), words(r#"echo "a""#));
}

#[test]
fn the_braces_are_a_spelling_and_the_name_is_the_node() {
    assert_eq!(parameter("echo $x"), parameter("echo ${x}"));
    assert_eq!(parameter("echo $x").name, "x");
    assert_eq!(parameter("echo $@").name, "@");
    assert_eq!(parameter("echo ${@}").name, "@");
    assert_eq!(parameter("echo $?").name, "?");
    assert_eq!(parameter("echo ${HOME}").name, "HOME");
    // ⚠ One digit unbraced: `$10` is `${1}` and a `0`. Settled by running bash,
    // because its printer spells both the same.
    assert_eq!(parameter("echo ${10}").name, "10");
    assert_eq!(
        segments("echo $10", 1),
        vec![
            Segment {
                kind: SegmentKind::Parameter(reader::syntax::ast::Parameter {
                    name: "1".into(),
                    quoted: false,
                    subscript: None,
                    op: None
                }),
                span: Span::new(0, 0)
            },
            Segment {
                kind: SegmentKind::Literal("0".into()),
                span: Span::new(0, 0)
            },
        ]
    );
}

#[test]
fn the_printer_braces_a_name_that_would_otherwise_run_on() {
    // Without this `${x}y` prints as `$xy`, which names a different parameter —
    // and re-reads as one, so the round-trip law is what catches it.
    assert_eq!(print(&tree("echo ${x}y")), "echo ${x}y");
    assert_eq!(print(&tree("echo ${10}")), "echo ${10}");
    // Nothing to run into, so no braces are needed.
    assert_eq!(print(&tree("echo ${x}")), "echo $x");
    assert_eq!(print(&tree("echo ${x}/y")), "echo $x/y");
    assert_eq!(print(&tree("echo ${x}*")), "echo $x*");
    // A special parameter cannot be extended, so it never needs them.
    assert_eq!(print(&tree("echo ${@}abc")), "echo $@abc");
}

#[test]
fn a_word_can_hold_text_and_parameters_together() {
    assert_eq!(
        segments("echo a$x", 1),
        vec![
            Segment {
                kind: SegmentKind::Literal("a".into()),
                span: Span::new(0, 0)
            },
            Segment {
                kind: SegmentKind::Parameter(reader::syntax::ast::Parameter {
                    name: "x".into(),
                    quoted: false,
                    subscript: None,
                    op: None
                }),
                span: Span::new(0, 0)
            },
        ]
    );
    // Inside quotes the run is several segments too, and the parameter carries
    // the quoting while the text around it does not need to.
    let mixed = segments(r#"echo "a$x b""#, 1);
    assert_eq!(mixed.len(), 3);
    assert!(matches!(&mixed[1].kind, SegmentKind::Parameter(p) if p.quoted && p.name == "x"));
}

#[test]
fn a_parameter_is_not_a_literal_and_cannot_be_read_as_one() {
    // `as_literal` is what the reserved-word check uses, so a word holding an
    // expansion must not answer it — otherwise `$x` could be mistaken for
    // grammar, or an expansion could be silently compared as text.
    assert_eq!(words("echo $x")[1].as_literal(), None);
    assert_eq!(words("echo a$x")[1].as_literal(), None);
}

#[test]
fn a_binding_survives_a_value_that_expands() {
    // ⚠ The regression this construct would otherwise have caused: with `$x` a
    // segment, `FOO=$x cmd` has no literal-only first word, so a check that read
    // the finished word would have skipped silently.

    // And a command whose NAME expands is not a binding.
    assert!(parse("$x=y").is_ok());
}

#[test]
fn an_expansion_with_no_closing_brace_is_refused() {
    // Bash refuses it too, so this is a claim about the input.
    assert_eq!(refusal("echo ${x"), Reason::UnterminatedExpansion);
}

#[test]
fn the_law_holds_across_the_parameter_shapes() {
    for text in [
        "echo $x",
        r#"echo "$x""#,
        "echo ${x}y",
        "echo a$x",
        "echo $x$y",
        r#"echo "a$x b""#,
        "echo $@",
        r#"echo "$@""#,
        "echo $1 $2",
        "echo ${10}",
        "echo $HOME/code",
        "cd $HOME",
        "echo $x*",
        r#"echo "$x"'y'"#,
        "cat <<EOF\n$x\nEOF",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
        assert!(
            survey(text).is_empty(),
            "{text:?} should need nothing: {:?}",
            survey(text)
        );
    }
}

#[test]
fn a_bracket_pair_split_across_segments_is_still_quoted() {
    // ⚠ Regression found by the round-trip law on 2 corpus commands. `[rc=` and
    // `]` each need no quoting alone, and printed bare they compose into
    // `[rc="$?"]` — which reads back as a bracket expression, not as this word.
    // The rule is per-word and the quoting decision was per-segment.
    assert!(check(r#"echo "[rc=$?]""#).holds());
    assert!(check(r#"echo "[a$x]""#).holds());
    assert!(check("echo '[a'$x']'").holds());
    // A `[` with no `]` after it still goes out bare, so the commonest
    // conditional in the corpus is untouched.
    assert_eq!(print(&tree("[ -f x ]")), "[ -f x ]");
    assert_eq!(print(&tree("echo [rc=")), "echo [rc=");
}

// ---- assignments ----

fn assignments(text: &str) -> Vec<reader::syntax::ast::Assignment> {
    simple(text).assignments.clone()
}

#[test]
fn a_binding_is_a_prefix_and_stops_at_the_command_name() {
    // ⚠ `A=1 cmd B=2` binds A and passes `B=2` as an argument — bash prints
    // exactly that back, so the prefix ends at the first word.
    let bound = assignments("A=1 cmd B=2");
    assert_eq!(bound.len(), 1);
    assert_eq!(bound[0].name, "A");
    assert_eq!(words("A=1 cmd B=2").len(), 2);
    assert_eq!(words("A=1 cmd B=2")[1].as_literal().as_deref(), Some("B=2"));
    // Several bind in order, and a command is optional.
    assert_eq!(
        assignments("A=1 B=2 cmd")
            .iter()
            .map(|a| a.name.clone())
            .collect::<Vec<_>>(),
        ["A", "B"]
    );
    assert!(words("A=1 B=2").is_empty());
    // `export FOO=bar` binds nothing: the word after the command name is an
    // argument that happens to look like one.
    assert!(assignments("export FOO=bar").is_empty());
}

#[test]
fn a_value_does_not_glob_and_an_argument_does() {
    // ⚠ Measured: `FOO=*.txt` binds those five characters, while `cmd *.txt`
    // names files. Recording a `Glob` in a value would claim an expansion the
    // shell does not do, and no gate could see it — bash prints both verbatim.
    assert_eq!(
        assignments("FOO=*.txt")[0].value.as_literal().as_deref(),
        Some("*.txt")
    );
    assert_eq!(words("cmd *.txt")[1].as_literal(), None);
    assert!(check("FOO=*.txt").holds());
}

#[test]
fn a_value_expands_a_tilde_after_a_colon() {
    // ⚠ The other half of why a value cannot share the argument reader: bash
    // binds `T=a:~/x` to `a:/home/…/x`, and no argument would.
    let value = &assignments("PATH=a:~/bin")[0].value;
    assert!(
        value
            .segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::Tilde(_))),
        "no tilde in {value:?}"
    );
    // At the head it expands in both kinds.
    assert!(matches!(
        assignments("HOME=~/x")[0].value.segments[0].kind,
        SegmentKind::Tilde(_)
    ));
    // ...and in an argument, a tilde after a colon is ordinary text.
    assert_eq!(
        words("cmd a:~/bin")[1].as_literal().as_deref(),
        Some("a:~/bin")
    );
}

#[test]
fn an_empty_value_is_one_tree_however_it_is_spelled() {
    // Both bind the empty string, so they are one node — and they have to be, or
    // the printer's single spelling would fail the law on the other.
    assert_eq!(assignments("FOO="), assignments("FOO=''"));
    assert!(assignments("FOO=")[0].value.segments.is_empty());
    assert_eq!(print(&tree("FOO=''")), "FOO=");
}

#[test]
fn appending_is_not_binding() {
    assert!(assignments("FOO+=bar")[0].append);
    assert!(!assignments("FOO=bar")[0].append);
    assert_ne!(assignments("FOO+=bar"), assignments("FOO=bar"));
    assert_eq!(print(&tree("FOO+=bar")), "FOO+=bar");
}

#[test]
fn the_law_holds_across_the_assignment_shapes() {
    for text in [
        "FOO=bar cmd",
        "FOO=bar",
        "FOO=",
        "FOO+=bar cmd",
        "A=1 B=2 cmd arg",
        r#"FOO="a b" cmd"#,
        "FOO=$x cmd",
        "PATH=$HOME/bin:$PATH",
        "PATH=~/bin:$PATH",
        "FOO=*.txt",
        "A=1 cmd B=2",
        "FOO=bar cmd > out",
        "FOO=bar cmd | wc -l",
        "A=1 cmd && B=2 other",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
        assert!(
            survey(text).is_empty(),
            "{text:?} should need nothing: {:?}",
            survey(text)
        );
    }
}

#[test]
fn each_reserved_word_belongs_to_the_construct_it_opens() {
    // ⚠ A reason is a unit of work, and these keywords are five grammars, not
    // one: counting them together would say how many commands hold a keyword,
    // which is not a number anything can be built against. `if` was in this
    // list until the conditional was built, and the split is what said it was
    // worth 50 commands where the loops were worth 4,573.
    assert!(parse("if a; then b; fi").is_ok());
    assert_eq!(refusal("case $x in a) b;; esac"), Reason::Case);
    assert_eq!(refusal("[[ -f x ]]"), Reason::TestExpression);
    assert_eq!(refusal("function f { a; }"), Reason::FunctionDefinition);
    assert_eq!(refusal("coproc a"), Reason::Coproc);
    // `!` is grammar at a pipeline's head and a syntax error elsewhere, so it
    // is refused in both places but for different reasons.
    assert_eq!(refusal("a | ! b"), Reason::MisplacedNegation);
    assert!(parse("! a").is_ok());
    // `[[` is a language; `[` is the test builtin and stays a command.
    assert!(parse("[ -f x ]").is_ok());
}

// ---- loops ----

fn loop_of(text: &str) -> reader::syntax::ast::CommandKind {
    match &list(text).first.commands[..] {
        [command] => command.kind.clone(),
        other => panic!("{text:?} is not one command: {other:?}"),
    }
}

fn for_loop(text: &str) -> reader::syntax::ast::ForLoop {
    match loop_of(text) {
        reader::syntax::ast::CommandKind::For(l) => l,
        other => panic!("{text:?} is not a for loop: {other:?}"),
    }
}

fn while_loop(text: &str) -> reader::syntax::ast::WhileLoop {
    match loop_of(text) {
        reader::syntax::ast::CommandKind::While(l) => l,
        other => panic!("{text:?} is not a while loop: {other:?}"),
    }
}

#[test]
fn a_body_is_a_command_list_and_layout_is_not_recorded() {
    // The body is read by the same reader a whole script is, so the two
    // spellings of one loop are one tree.
    assert_eq!(
        for_loop("for f in a b; do echo $f; done"),
        for_loop("for f in a b\ndo\n  echo $f\ndone")
    );
    let loop_ = for_loop("for f in a b; do x; y; done");
    assert_eq!(loop_.name, "f");
    assert_eq!(loop_.words.len(), 2);
    assert_eq!(loop_.body.len(), 2);
}

#[test]
fn an_omitted_list_is_desugared_the_way_bash_desugars_it() {
    // ⚠ `for f; do …` comes back from `declare -f` as `for f in "$@"; do …`, so
    // the tree holds the explicit list. Recording the omission would make one
    // command two trees, and the second gate would say so.
    assert_eq!(
        for_loop("for f; do x; done"),
        for_loop(r#"for f in "$@"; do x; done"#)
    );
    assert_eq!(
        print(&tree("for f; do x; done")),
        r#"for f in "$@"; do x; done"#
    );
}

#[test]
fn until_is_while_with_the_sense_reversed_and_select_is_not_for() {
    assert!(while_loop("until a; do b; done").until);
    assert!(!while_loop("while a; do b; done").until);
    assert_ne!(
        while_loop("until a; do b; done"),
        while_loop("while a; do b; done")
    );
    assert!(for_loop("select f in a; do b; done").select);
    assert!(!for_loop("for f in a; do b; done").select);
}

#[test]
fn a_condition_is_a_list_not_a_command() {
    // `while read -r a && test x; do` is legal, so the condition holds a list.
    let loop_ = while_loop("while a && b; do c; done");
    assert_eq!(loop_.condition.len(), 1);
    assert!(check("while a && b; do c; done").holds());
}

#[test]
fn a_loop_takes_its_redirections_after_done() {
    // ⚠ Bash prints them there, and it says so structurally by moving one:
    // `for f in a; do b; done > out`.
    assert_eq!(redirects("for f in a; do b; done > out").len(), 1);
    assert!(check("for f in a; do b; done > out").holds());
    assert!(check("while a; do b; done 2>&1").holds());
}

#[test]
fn a_loop_is_a_command_in_a_pipeline() {
    let piped = list("for f in a; do b; done | wc -l");
    assert_eq!(piped.first.commands.len(), 2);
    assert!(check("for f in a; do b; done | wc -l").holds());
}

#[test]
fn a_heredoc_inside_a_body_still_finds_its_body() {
    // The fill is positional, so a compound's interior has to be walked before
    // its redirections — the body text comes first.
    let loop_ = for_loop("for f in a; do cat <<EOF\nx\nEOF\ndone");
    assert_eq!(loop_.body.len(), 1);
    assert!(check("for f in a; do cat <<EOF\nx\nEOF\ndone").holds());
}

#[test]
fn a_comment_in_a_body_is_refused_rather_than_dropped() {
    // The printer puts a loop on one line, where a comment would swallow the
    // rest of it. The same answer this tree gives a comment in an and-or list.
    assert_eq!(
        refusal("for f in a; do\n# note\nb\ndone"),
        Reason::CommentInList
    );
}

#[test]
fn the_arithmetic_for_is_a_different_grammar() {
    assert_eq!(
        refusal("for ((i=0;i<3;i++)); do b; done"),
        Reason::Arithmetic
    );
}

#[test]
fn the_law_holds_across_the_loop_shapes() {
    for text in [
        "for f in a b; do echo $f; done",
        "for f in a b\ndo\necho $f\ndone",
        "for f; do x; done",
        "for f in *.txt; do x; done",
        "while read -r l; do echo $l; done",
        "until a; do b; done",
        "select f in a b; do echo $f; done",
        "for f in a; do b; c; done",
        "for f in a; do b; done > out",
        "for f in a; do b; done | wc -l",
        "while a && b; do c; done",
        "for f in a; do for g in b; do c; done; done",
        "for f in a; do b & done",
        "for f in a; do cat <<EOF\nx\nEOF\ndone",
        "cd x && for f in a; do b; done",
        "for f in a; do FOO=1 b; done",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
        assert!(
            survey(text).is_empty(),
            "{text:?} should need nothing: {:?}",
            survey(text)
        );
    }
}

// ---- command substitution ----

fn substitution(text: &str) -> reader::syntax::ast::Substitution {
    match &segments(text, 1)[..] {
        [
            Segment {
                kind: SegmentKind::Substitution(s),
                ..
            },
        ] => s.clone(),
        other => panic!("{text:?} is not one substitution: {other:?}"),
    }
}

#[test]
fn a_substitution_holds_a_script_the_gates_can_see_into() {
    // ⚠ Unlike a word, bash NORMALISES what is inside: `$(a|b)` comes back as
    // `$(a | b)` and `$(ls |& cat)` as `$(ls 2>&1 | cat)`. So the interior is a
    // real parse on both sides of the second gate, and a misparse in there
    // would be caught rather than printed straight back.
    let inner = substitution("echo $(a | b)");
    assert_eq!(inner.items.len(), 1);
    assert!(!inner.quoted);
    // Layout inside collapses exactly as it does outside.
    assert_eq!(
        substitution("echo $(a|b)"),
        substitution("echo $(a   |   b)")
    );
    assert_eq!(substitution("echo $(a; b)").items.len(), 2);
}

#[test]
fn quoting_a_substitution_is_semantic() {
    assert!(substitution(r#"echo "$(a)""#).quoted);
    assert!(!substitution("echo $(a)").quoted);
    assert_ne!(words("echo $(a)"), words(r#"echo "$(a)""#));
}

#[test]
fn a_substitution_nests_and_sits_inside_a_word() {
    assert!(check("echo $(a $(b))").holds());
    assert_eq!(segments("echo x$(a)y", 1).len(), 3);
    // ...and reaches the places a word can reach.
    assert!(check("FOO=$(a) cmd").holds());
    assert!(check("$(a) arg").holds());
    assert!(check("for f in $(ls); do b; done").holds());
}

#[test]
fn a_backtick_is_a_different_build_and_stays_refused() {
    // ⚠ Bash prints a backtick's interior VERBATIM — `` `a|b` `` stays as
    // written — where it normalises `$( )`. Different rendering, different
    // escaping, and 18 corpus commands against 6397.
    assert_eq!(refusal("echo `a|b`"), Reason::Backtick);
    assert!(survey("echo `a|b`").contains(&Reason::Backtick));
}

#[test]
fn what_a_substitution_cannot_carry_is_named() {
    // A heredoc's body would have to sit on the lines after the opener, and a
    // substitution prints inline — there is nowhere for it to go.
    assert_eq!(
        refusal("echo $(cat <<EOF\nx\nEOF\n)"),
        Reason::CommandSubstitution
    );
    assert!(survey("echo $(cat <<EOF\nx\nEOF\n)").contains(&Reason::CommandSubstitution));
    // A comment would swallow the rest of the inline form.
    assert_eq!(refusal("echo $(a # note\n)"), Reason::CommentInList);
    // An unclosed one is a syntax error to bash too.
    assert_eq!(refusal("echo $(a"), Reason::UnterminatedExpansion);
}

#[test]
fn the_law_holds_across_the_substitution_shapes() {
    for text in [
        "echo $(a)",
        r#"echo "$(a)""#,
        "echo $(a | b)",
        "echo $(a; b)",
        "echo $(a && b)",
        "echo x$(a)y",
        "echo $(a $(b))",
        "FOO=$(a) cmd",
        "$(a) arg",
        "cd $(dirname $0)",
        "for f in $(ls); do echo $f; done",
        "echo $(for f in a; do b; done)",
        r#"echo "$(a)$(b)""#,
        "echo $(a > out)",
        "echo $(cat f | wc -l) lines",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
        assert!(
            survey(text).is_empty(),
            "{text:?} should need nothing: {:?}",
            survey(text)
        );
    }
}

// ---- conditionals ----

fn conditional(text: &str) -> reader::syntax::ast::Conditional {
    match loop_of(text) {
        reader::syntax::ast::CommandKind::If(c) => c,
        other => panic!("{text:?} is not a conditional: {other:?}"),
    }
}

#[test]
fn elif_is_desugared_the_way_bash_desugars_it() {
    // ⚠ Measured in `reader/probes/conditional.sh`: bash prints
    // `if a; then b; elif c; then d; fi` back as
    // `if a; then b; else if c; then d; fi; fi`. So an `elif` is an `else`
    // holding one nested conditional, and a tree with a list of arms would make
    // those two texts two trees — which the second gate would report.
    assert_eq!(
        conditional("if a; then b; elif c; then d; fi"),
        conditional("if a; then b; else if c; then d; fi; fi")
    );
    assert_eq!(
        print(&tree("if a; then b; elif c; then d; else e; fi")),
        "if a; then b; else if c; then d; else e; fi; fi"
    );
}

#[test]
fn a_conditional_is_three_lists_and_layout_is_not_recorded() {
    assert_eq!(
        conditional("if a; then b; fi"),
        conditional("if a\nthen\n  b\nfi")
    );
    let c = conditional("if a; b; then c; d; else e; fi");
    // ⚠ The condition is a LIST whose last status decides the branch, not one
    // command: `if a; b; then` runs both and tests `b`.
    assert_eq!(c.condition.len(), 2);
    assert_eq!(c.then.len(), 2);
    assert_eq!(c.otherwise.map(|items| items.len()), Some(1));
    assert!(conditional("if a; then b; fi").otherwise.is_none());
}

#[test]
fn an_empty_arm_is_a_syntax_error_not_an_empty_list() {
    // Bash refuses all three, so these are claims about the input and `bash -n`
    // adjudicates them.
    assert_eq!(refusal("if a; then fi"), Reason::EmptyOperand);
    assert_eq!(refusal("if; then b; fi"), Reason::EmptyOperand);
    assert_eq!(refusal("if a; then b; else fi"), Reason::EmptyOperand);
}

#[test]
fn a_conditional_that_does_not_close_is_refused_by_name() {
    assert_eq!(refusal("if a; then b"), Reason::Conditional);
    assert_eq!(refusal("if a then b; fi"), Reason::Conditional);
    assert_eq!(refusal("if a; then b; elif c; fi"), Reason::Conditional);
    // A branch keyword with no `if` open is refused where it stands.
    assert_eq!(refusal("fi"), Reason::Conditional);
    assert_eq!(refusal("then b"), Reason::Conditional);
    // ⚠ Each of those is in the survey's set too, or the invariant that pins
    // the two scanners together would be broken.
    for text in [
        "if a; then b",
        "if a then b; fi",
        "if a; then b; elif c; fi",
        "fi",
        "then b",
    ] {
        assert!(
            survey(text).contains(&Reason::Conditional),
            "the survey missed the conditional in {text:?}: {:?}",
            survey(text)
        );
    }
}

#[test]
fn a_quoted_keyword_is_a_program_and_stays_one() {
    // `'if' a` runs a program called `if`; bash prints the quotes straight
    // back, so neither gate can see a tree that confused the two.
    assert!(parse("'if' a").is_ok());
    assert_eq!(print(&tree("'if' a")), "'if' a");
    assert!(check("'if' a").holds());
}

#[test]
fn a_comment_inside_a_conditional_is_refused_rather_than_dropped() {
    // The printer puts a conditional on one line, where a comment would swallow
    // the rest of it. Bash deletes comments, so it has no opinion — the same
    // answer a loop body's comment gets.
    assert_eq!(refusal("if a; then\n# note\nb\nfi"), Reason::CommentInList);
    assert!(survey("if a; then\n# note\nb\nfi").contains(&Reason::CommentInList));
}

#[test]
fn a_body_ending_in_an_ampersand_takes_no_semicolon_after_it() {
    // ⚠ Measured: `if a; then b & fi` is legal and `if a; then b & ; fi` is a
    // syntax error. The printer emitted the second for every compound whose
    // body ended in a `&` — invalid shell that BOTH gates passed, because gate
    // 1 re-reads it with this parser and gate 2 never sees our print.
    assert_eq!(print(&tree("if a; then b & fi")), "if a; then b & fi");
    assert_eq!(
        print(&tree("for f in a; do b & done")),
        "for f in a; do b & done"
    );
    assert_eq!(print(&tree("while a; do b & done")), "while a; do b & done");
}

#[test]
fn the_law_holds_across_the_conditional_shapes() {
    for text in [
        "if a; then b; fi",
        "if a\nthen\nb\nfi",
        "if a; then b; else c; fi",
        "if a; then b; elif c; then d; fi",
        "if a; then b; elif c; then d; elif e; then f; else g; fi",
        "if ! a; then b; fi",
        "if a && b || c; then d; fi",
        "if a | b; then c; fi",
        "if a; b; then c; fi",
        "if [ -f x ]; then b; fi",
        "if a; then b; c; fi",
        "if a; then b; fi > out",
        "if a; then b; fi | wc -l",
        "a | if b; then c; fi",
        "if a; then b & fi",
        "if a; then if b; then c; fi; fi",
        "if a; then for f in x; do y; done; fi",
        "for f in x; do if a; then b; fi; done",
        "if a; then cat <<EOF\nx\nEOF\nfi",
        "if a; then b; fi <<EOF\nx\nEOF",
        "cd x && if a; then b; fi",
        "if a; then FOO=1 b; fi",
        "echo $(if a; then b; fi)",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
        assert!(
            survey(text).is_empty(),
            "{text:?} should need nothing: {:?}",
            survey(text)
        );
    }
}

// ---- parameter operators ----

fn parameter_of(text: &str) -> reader::syntax::ast::Parameter {
    match &segments(text, 1)[..] {
        [
            Segment {
                kind: SegmentKind::Parameter(p),
                ..
            },
        ] => p.clone(),
        other => panic!("{text:?} is not one parameter: {other:?}"),
    }
}

#[test]
fn the_colon_is_a_field_because_it_changes_what_substitutes() {
    // ⚠ `${x-y}` substitutes only for an UNSET x; `${x:-y}` also for an empty
    // one. Bash prints both back as written, so nothing downstream would catch
    // these being collapsed — construction is the only defence.
    use reader::syntax::ast::ParameterOp;
    let with = parameter_of("echo ${x:-y}").op.expect("an operator");
    let without = parameter_of("echo ${x-y}").op.expect("an operator");
    assert!(matches!(with, ParameterOp::Default { colon: true, .. }));
    assert!(matches!(without, ParameterOp::Default { colon: false, .. }));
    assert_ne!(with, without);
    assert_eq!(print(&tree("echo ${x-y}")), "echo ${x-y}");
}

#[test]
fn each_operator_is_its_own_node_not_a_string() {
    use reader::syntax::ast::ParameterOp;
    assert!(matches!(
        parameter_of("echo ${x:=y}").op,
        Some(ParameterOp::Assign { .. })
    ));
    assert!(matches!(
        parameter_of("echo ${x:?y}").op,
        Some(ParameterOp::Error { .. })
    ));
    assert!(matches!(
        parameter_of("echo ${x:+y}").op,
        Some(ParameterOp::Alternate { .. })
    ));
    assert!(matches!(
        parameter_of("echo ${#x}").op,
        Some(ParameterOp::Length)
    ));
    assert!(matches!(
        parameter_of("echo ${!x}").op,
        Some(ParameterOp::Indirect)
    ));
    // The doubled forms take the LONGEST match, which is a different program.
    assert!(matches!(
        parameter_of("echo ${x#p}").op,
        Some(ParameterOp::StripPrefix { longest: false, .. })
    ));
    assert!(matches!(
        parameter_of("echo ${x##p}").op,
        Some(ParameterOp::StripPrefix { longest: true, .. })
    ));
    assert!(matches!(
        parameter_of("echo ${x%%s}").op,
        Some(ParameterOp::StripSuffix { longest: true, .. })
    ));
    assert!(matches!(
        parameter_of("echo ${x^^}").op,
        Some(ParameterOp::Case {
            upper: true,
            every: true
        })
    ));
}

#[test]
fn a_subscript_names_an_element_and_forces_the_braces() {
    use reader::syntax::ast::Subscript;
    let p = parameter_of("echo ${PIPESTATUS[0]}");
    assert_eq!(p.name, "PIPESTATUS");
    assert!(matches!(p.subscript, Some(Subscript::Index(_))));
    assert!(matches!(
        parameter_of("echo ${a[@]}").subscript,
        Some(Subscript::All)
    ));
    // ⚠ `[@]` and `[*]` differ the way `"$@"` and `"$*"` do — how many words.
    assert!(matches!(
        parameter_of("echo ${a[*]}").subscript,
        Some(Subscript::Joined)
    ));
    assert_ne!(parameter_of("echo ${a[@]}"), parameter_of("echo ${a[*]}"));
    // `$a[0]` is `$a` and the literal `[0]`: a different word entirely, so the
    // printer may never drop these braces.
    assert_eq!(print(&tree("echo ${a[0]}")), "echo ${a[0]}");
    // `$a[0]` is `$a` followed by `[0]`, which unquoted is a bracket
    // expression — a construct this tree does not model yet. That it is refused
    // rather than read as the subscript above is the point.
    assert_eq!(refusal("echo $a[0]"), Reason::BracketExpression);
}

#[test]
fn an_operand_nests_and_holds_spaces_bare() {
    // ⚠ Both measured: `${x:-$(date)}` means the operand cannot be found by
    // scanning to the first `}`, and `${x:-a b}` is ONE word, so it cannot stop
    // at a space either.
    assert!(parse("echo ${x:-$(date)}").is_ok());
    assert!(parse("echo ${x:-${y}}").is_ok());
    assert_eq!(print(&tree("echo ${x:-a b}")), "echo ${x:-a b}");
    assert!(check("echo ${x:-$(date)}").holds());
    assert!(check("echo ${x:-${y}}").holds());
}

#[test]
fn a_pattern_operand_is_a_pattern_not_literal_text() {
    // `${f%%.*}` cuts at the FIRST dot: the `*` is the same glob language
    // pathname expansion uses, so it is a Glob segment rather than an asterisk.
    use reader::syntax::ast::{Glob, ParameterOp};
    let Some(ParameterOp::StripSuffix { pattern, .. }) = parameter_of("echo ${f%%.*}").op else {
        panic!("not a suffix strip");
    };
    assert!(
        pattern
            .segments
            .iter()
            .any(|s| matches!(s.kind, SegmentKind::Glob(Glob::Any))),
        "the `*` in a pattern must be a glob: {pattern:?}"
    );
}

#[test]
fn a_replacement_is_absent_rather_than_empty_when_none_was_written() {
    use reader::syntax::ast::{Anchor, ParameterOp};
    let Some(ParameterOp::Replace(r)) = parameter_of("echo ${x/a/b}").op else {
        panic!("not a replace");
    };
    assert!(!r.every && r.anchor.is_none() && r.replacement.is_some());
    let Some(ParameterOp::Replace(all)) = parameter_of("echo ${x//a/b}").op else {
        panic!("not a replace");
    };
    assert!(all.every);
    let Some(ParameterOp::Replace(anchored)) = parameter_of("echo ${x/#a/b}").op else {
        panic!("not a replace");
    };
    assert_eq!(anchored.anchor, Some(Anchor::Start));
    // `${x/a}` deletes the match and gives no replacement to read.
    let Some(ParameterOp::Replace(cut)) = parameter_of("echo ${x/a}").op else {
        panic!("not a replace");
    };
    assert!(cut.replacement.is_none());
}

#[test]
fn what_this_build_does_not_reach_is_still_refused_by_name() {
    // ⚠ A substring takes ARITHMETIC on both sides, which is a language of its
    // own — so it stays refused rather than being read as text.
    assert_eq!(refusal("echo ${x:1:3}"), Reason::ParameterOperator);
    assert!(survey("echo ${x:1:3}").contains(&Reason::ParameterOperator));
    // An arithmetic index is refused as arithmetic, which is what it is.
    assert_eq!(refusal("echo ${a[i+1]}"), Reason::Arithmetic);
    assert_eq!(refusal("echo ${}"), Reason::ParameterOperator);
    // Bash refuses this one too, so `bash -n` adjudicates the claim.
    assert_eq!(refusal("echo ${x"), Reason::UnterminatedExpansion);
}

#[test]
fn the_law_holds_across_the_parameter_operator_shapes() {
    for text in [
        "echo ${x:-y}",
        "echo ${x-y}",
        "echo ${x:=y}",
        "echo ${x:?msg}",
        "echo ${x:+y}",
        "echo ${#x}",
        "echo ${!x}",
        "echo ${x#p}",
        "echo ${x##*/}",
        "echo ${x%s}",
        "echo ${f%%.*}",
        "echo ${x/a/b}",
        "echo ${x//a/b}",
        "echo ${x/#a/b}",
        "echo ${x/%a/b}",
        "echo ${x/a}",
        "echo ${x^}",
        "echo ${x,,}",
        "echo ${PIPESTATUS[0]}",
        "echo ${a[@]}",
        "echo ${#a[@]}",
        "echo ${a[$i]}",
        r#"echo "${x:-y}""#,
        "echo ${x:-a b}",
        "echo ${x:-$(date)}",
        "echo ${x:-${y}}",
        "echo ${x:-}",
        "echo ${x:-*}",
        "echo ${x:-a/b}",
        "FOO=${x:-y} cmd",
        "for f in ${a[@]}; do echo ${f%%.*}; done",
        "if [ -n ${x:-} ]; then echo ${#x}; fi",
    ] {
        assert!(
            check(text).holds(),
            "the law failed on {text:?}: {}",
            check(text).label()
        );
        assert!(
            survey(text).is_empty(),
            "{text:?} should need nothing: {:?}",
            survey(text)
        );
    }
}
