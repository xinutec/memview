//! What a command will leave in the files it writes, predicted from its text and
//! the state it is given.

use std::collections::BTreeMap;

use reader::predict::{
    Alternatives, Assumed, Conditional, Dirs, Files, Prediction, Shown, Unfollowed, Why, Written,
    needs, predict,
};

const HOME: &str = "/home/me";
const CWD: &str = "/repo";

fn run(script: &str, files: &Files) -> Prediction {
    let parsed = reader::syntax::parse(script).expect("parses");
    predict(&parsed, CWD, HOME, files)
}

fn written(path: &str, text: &str) -> Written {
    Written {
        path: path.to_string(),
        text: Some(text.to_string()),
    }
}

/// A file the command removes.
fn removed(path: &str) -> Written {
    Written {
        path: path.to_string(),
        text: None,
    }
}

fn nothing_known() -> Files {
    Files::new()
}

fn known(pairs: &[(&str, Option<&str>)]) -> Files {
    pairs
        .iter()
        .map(|(path, text)| (path.to_string(), text.map(str::to_string)))
        .collect::<BTreeMap<_, _>>()
}

#[test]
fn a_heredoc_into_a_file_is_its_whole_text() {
    let found = run(
        "cat > notes.txt <<'EOF'\nfirst\nsecond\nEOF",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![written("/repo/notes.txt", "first\nsecond\n")]
    );
    assert!(found.unfollowed.is_empty());
}

#[test]
fn the_redirect_may_follow_the_heredoc() {
    let found = run("cat <<'EOF' > /tmp/a\nx\nEOF", &nothing_known());
    assert_eq!(found.written, vec![written("/tmp/a", "x\n")]);
}

#[test]
fn an_unquoted_heredoc_without_expansions_is_literal() {
    let found = run("cat > a <<EOF\nplain text\nEOF", &nothing_known());
    assert_eq!(found.written, vec![written("/repo/a", "plain text\n")]);
}

#[test]
fn an_unquoted_heredoc_that_expands_is_not_followed() {
    let found = run("cat > a <<EOF\nuser is $USER\nEOF", &nothing_known());
    assert!(found.written.is_empty());
    assert_eq!(found.unfollowed.len(), 1);
    // `HOME` the run carries, as it does in a word.
    let found = run("cat > a <<EOF\nhome is $HOME\nEOF", &nothing_known());
    assert_eq!(
        found.written,
        vec![written("/repo/a", "home is /home/me\n")]
    );
}

#[test]
fn echo_writes_its_words_and_a_newline() {
    let found = run("echo hello world > a && echo -n more > b", &nothing_known());
    assert_eq!(
        found.written,
        vec![
            written("/repo/a", "hello world\n"),
            written("/repo/b", "more")
        ]
    );
}

#[test]
fn printf_writes_its_format() {
    let found = run(r"printf 'a\tb\n100%%\n' > a", &nothing_known());
    assert_eq!(found.written, vec![written("/repo/a", "a\tb\n100%\n")]);
}

#[test]
fn printf_with_a_directive_is_not_followed() {
    let found = run("printf '%s\\n' x > a", &nothing_known());
    assert!(found.written.is_empty());
}

#[test]
fn an_append_needs_what_the_file_held() {
    let script = "echo two >> a";
    let parsed = reader::syntax::parse(script).expect("parses");
    assert_eq!(needs(&parsed, CWD, HOME), vec!["/repo/a".to_string()]);
    let found = run(script, &known(&[("/repo/a", Some("one\n"))]));
    assert_eq!(found.written, vec![written("/repo/a", "one\ntwo\n")]);
}

#[test]
fn an_append_to_a_file_that_is_not_there_creates_it() {
    let found = run("echo two >> a", &known(&[("/repo/a", None)]));
    assert_eq!(found.written, vec![written("/repo/a", "two\n")]);
}

#[test]
fn an_append_to_a_file_nobody_read_is_not_followed() {
    let found = run("echo two >> a", &nothing_known());
    assert!(found.written.is_empty());
    assert!(matches!(found.unfollowed.as_slice(), [Unfollowed { .. }]));
}

#[test]
fn writes_to_one_file_follow_each_other() {
    let found = run("echo one > a; echo two >> a", &nothing_known());
    assert_eq!(found.written, vec![written("/repo/a", "one\ntwo\n")]);
}

#[test]
fn a_cd_moves_where_relative_paths_land() {
    let found = run("cd sub && echo x > a", &nothing_known());
    assert_eq!(found.written, vec![written("/repo/sub/a", "x\n")]);
}

#[test]
fn tee_writes_its_input_to_every_file_it_names() {
    let found = run("tee a b <<'EOF' > /dev/null\nbody\nEOF", &nothing_known());
    assert_eq!(
        found.written,
        vec![written("/repo/a", "body\n"), written("/repo/b", "body\n")]
    );
}

#[test]
fn cat_of_known_files_concatenates_them() {
    let script = "cat a b > c";
    let parsed = reader::syntax::parse(script).expect("parses");
    assert_eq!(
        needs(&parsed, CWD, HOME),
        vec!["/repo/a".to_string(), "/repo/b".to_string()]
    );
    let found = run(
        script,
        &known(&[("/repo/a", Some("1\n")), ("/repo/b", Some("2\n"))]),
    );
    assert_eq!(found.written, vec![written("/repo/c", "1\n2\n")]);
}

#[test]
fn an_unknown_program_writing_a_file_is_not_followed_and_forgets_it() {
    let found = run("echo one > a; make > a", &nothing_known());
    assert!(found.written.is_empty(), "what `make` printed is not known");
    assert_eq!(found.unfollowed.len(), 1);
}

#[test]
fn a_pipeline_into_a_file_is_not_followed() {
    let found = run("echo x | sort > a", &nothing_known());
    assert!(found.written.is_empty());
    assert_eq!(found.unfollowed.len(), 1);
}

/// An `if` this cannot decide runs both arms, and a file they leave
/// differently is one of their texts: exact as a set, each member from an arm
/// the text has. Reading such a file for its text is refused.
#[test]
fn an_undecided_if_leaves_a_file_one_of_its_arms_texts() {
    let shown = known(&[("/repo/f", Some("x\n")), ("/repo/o", None)]);
    let set = |texts: &[Option<&str>]| Alternatives {
        path: "/repo/o".to_string(),
        texts: texts.iter().map(|t| t.map(str::to_string)).collect(),
    };
    for (command, texts) in [
        (
            "if [ -r f ]; then echo a > o; else echo b > o; fi",
            vec![Some("a\n"), Some("b\n")],
        ),
        ("if [ -r f ]; then echo a > o; fi", vec![None, Some("a\n")]),
        (
            "if [ -r f ]; then echo a > o; else echo b > o; fi; echo c >> o",
            vec![Some("a\nc\n"), Some("b\nc\n")],
        ),
        (
            "if [ -r f ]; then echo a > o; elif [ -w f ]; then echo b > o; else echo c > o; fi",
            vec![Some("b\n"), Some("c\n"), Some("a\n")],
        ),
    ] {
        let found = run(command, &shown);
        let mut got = found.alternatives.clone();
        for alternative in &mut got {
            alternative.texts.sort();
        }
        let mut want = set(&texts);
        want.texts.sort();
        assert_eq!(got, vec![want], "{command}: {:?}", found.unfollowed);
        assert!(
            !found.written.iter().any(|w| w.path == "/repo/o"),
            "{command}"
        );
        assert!(
            found.unfollowed.is_empty(),
            "{command}: {:?}",
            found.unfollowed
        );
    }
    let same = run("if [ -r f ]; then echo a > o; else echo a > o; fi", &shown);
    assert_eq!(same.written, vec![written("/repo/o", "a\n")]);

    let read = run("if [ -r f ]; then echo a > o; fi; cat o > p", &shown);
    assert!(
        read.unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/p") && u.why == Why::Branches)
    );

    let bound = run("if [ -r f ]; then v=a; else v=b; fi; echo z > $v", &shown);
    assert!(
        bound
            .written
            .iter()
            .all(|w| w.path != "/repo/a" && w.path != "/repo/b"),
        "{:?}",
        bound.written
    );

    let stops = run("if [ -r f ]; then exit; fi; echo a > o", &shown);
    assert!(
        !stops.written.iter().any(|w| w.path == "/repo/o"),
        "{:?}",
        stops.written
    );
    assert!(stops.alternatives.is_empty());
    // An exit that fails the call leaves it unchecked: the success the
    // prediction assumes is the world where it was not taken.
    for command in [
        "if [ -r f ]; then exit 1; fi; echo a > o",
        "[ -r f ] || exit 1; echo a > o",
    ] {
        assert_eq!(
            run(command, &shown).written,
            vec![written("/repo/o", "a\n")],
            "{command}"
        );
    }
    let bare = run("[ -r f ] || exit; echo a > o", &shown);
    assert!(bare.written.is_empty(), "{:?}", bare.written);
    let defined = run("f() { if [ -r f ]; then return; fi; }; echo a > o", &shown);
    assert_eq!(
        defined.written,
        vec![written("/repo/o", "a\n")],
        "a definition runs nothing"
    );
}

/// A Python `if` this cannot decide runs both arms: a file they leave
/// differently is one of their texts, a name they bind differently is
/// unknown, and arms that end differently leave the `if` refused.
#[test]
fn an_undecided_python_if_leaves_a_file_one_of_its_arms_texts() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[("/repo/o", Some("old"))]);
    let head = "import os, sys\nflag = os.environ.get('F')\n";
    let set = |found: &Prediction| -> Vec<Option<String>> {
        let mut texts = found
            .alternatives
            .iter()
            .find(|set| set.path == "/repo/o")
            .map(|set| set.texts.clone())
            .unwrap_or_default();
        texts.sort();
        texts
    };
    let some = |texts: &[&str]| {
        texts
            .iter()
            .map(|t| Some(t.to_string()))
            .collect::<Vec<_>>()
    };
    let both = run(
        &py(&format!(
            "{head}if flag:\n    open('o', 'w').write('a')\nelse:\n    open('o', 'w').write('b')"
        )),
        &shown,
    );
    assert_eq!(set(&both), some(&["a", "b"]), "{:?}", both.unfollowed);
    let one = run(
        &py(&format!("{head}if flag:\n    open('o', 'w').write('a')")),
        &shown,
    );
    assert_eq!(set(&one), some(&["a", "old"]));
    let same = run(
        &py(&format!(
            "{head}x = 'n'\nif flag:\n    x = 'n'\nopen('o', 'w').write(x)"
        )),
        &shown,
    );
    assert_eq!(same.written, vec![written("/repo/o", "n")]);
    let bound = run(
        &py(&format!(
            "{head}x = 'n'\nif flag:\n    x = 'm'\nopen('o', 'w').write(x)"
        )),
        &shown,
    );
    assert!(
        bound.written.is_empty() && bound.alternatives.is_empty(),
        "{:?}",
        bound.written
    );
    let raised = run(
        &py(&format!(
            "{head}if flag:\n    open('o', 'w').write('a')\n    raise SystemExit(0)\n"
        )),
        &shown,
    );
    assert!(raised.alternatives.is_empty(), "{:?}", raised.alternatives);
}

/// A jump in a Python block not followed makes what follows only sometimes
/// run, until the loop or the function it leaves ends; an exit that fails the
/// call, or a raise, is excluded by the success the prediction assumes.
#[test]
fn a_python_jump_not_followed_makes_what_follows_sometimes() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let head = "import os, sys\nflag = os.environ.get('F')\n";
    for program in [
        "if flag:\n    sys.exit(0)\nopen('o', 'w').write('x')",
        "if flag:\n    sys.exit()\nopen('o', 'w').write('x')",
        "for i in range(3):\n    if flag:\n        break\n    open('o', 'a').write('x')",
        "def f():\n    if flag:\n        return 'a'\n    return 'b'\nopen('o', 'w').write(f())",
    ] {
        let found = run(&py(&format!("{head}{program}")), &nothing_known());
        assert!(found.written.is_empty(), "{program}: {:?}", found.written);
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/o")),
            "{program}"
        );
    }
    for program in [
        "if flag:\n    sys.exit(1)\nopen('o', 'w').write('x')",
        "if flag:\n    raise SystemExit('no')\nopen('o', 'w').write('x')",
        "for i in range(2):\n    if flag:\n        continue\nopen('o', 'w').write('x')",
        "def f():\n    if flag:\n        return\nf()\nopen('o', 'w').write('x')",
    ] {
        let found = run(&py(&format!("{head}{program}")), &nothing_known());
        assert_eq!(
            found.written,
            vec![written("/repo/o", "x")],
            "{program}: {:?}",
            found.unfollowed
        );
    }
}

/// What a word runs as it expands runs before the command: a `$( )` is
/// followed as a subshell, and a program inside one may write any file.
/// Found in history: `n=$(nix develop -c npx eslint …)` in a loop, whose
/// later iterations could rewrite the files earlier ones removed.
#[test]
fn a_command_substitution_runs_before_its_command() {
    let written_there = run("x=$(echo a > f; echo b); echo c > g", &nothing_known());
    assert_eq!(
        written_there.written,
        vec![written("/repo/f", "a\n"), written("/repo/g", "c\n")]
    );

    let stray = run("echo x > f; n=$(./tool)", &nothing_known());
    assert!(stray.written.is_empty(), "{:?}", stray.written);
    assert!(
        stray
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/f"))
    );

    let looped = run(
        "for d in a b; do echo x > $d.p; n=$(./lint $d || true); rm -f $d.p; done",
        &nothing_known(),
    );
    assert!(
        !looped.written.iter().any(|w| w.path == "/repo/a.p"),
        "{:?}",
        looped.written
    );

    let maybe = run("echo x > f; y=${UNSET:-$(echo z > f)}", &nothing_known());
    assert!(
        !maybe.written.iter().any(|w| w.path == "/repo/f"),
        "{:?}",
        maybe.written
    );

    let heredoc = run(
        "echo x > f; cat > g <<EOF\n$(./tool)\nEOF",
        &nothing_known(),
    );
    assert!(
        !heredoc.written.iter().any(|w| w.path == "/repo/f"),
        "{:?}",
        heredoc.written
    );
}

/// `cd -` returns to a directory this does not keep, and `cd -P dir` goes to
/// `dir`: neither is a directory named for its flag.
#[test]
fn cd_flags_are_not_directories() {
    let back = run("cd sub; cd -; echo x > a", &nothing_known());
    assert!(
        !back
            .written
            .iter()
            .any(|w| w.path == "/repo/sub/-/a" || w.path == "/repo/-/a"),
        "{:?}",
        back.written
    );
    let physical = run("cd -P sub; echo x > a", &nothing_known());
    assert_eq!(physical.written, vec![written("/repo/sub/a", "x\n")]);
}

/// A `case` runs the arm its word selects; with the word unknown, each arm
/// and no arm are its outcomes.
#[test]
fn a_case_runs_the_arm_its_word_selects() {
    let shown = known(&[("/repo/o", Some("old\n"))]);
    let decided = run(
        "x=b.txt\ncase $x in a*) echo a > o;; *.txt) echo t > o;; *) echo z > o;; esac",
        &shown,
    );
    assert_eq!(decided.written, vec![written("/repo/o", "t\n")]);
    let none = run("case q in a) echo a > o;; esac; echo n > p", &shown);
    assert_eq!(none.written, vec![written("/repo/p", "n\n")]);
    let undecided = run(
        "case \"$UNSET\" in a) echo a > o;; b) echo b > o;; esac",
        &shown,
    );
    let mut texts = undecided.alternatives[0].texts.clone();
    texts.sort();
    assert_eq!(
        texts,
        vec![
            Some("a\n".to_string()),
            Some("b\n".to_string()),
            Some("old\n".to_string())
        ]
    );
    let every = run(
        "case \"$UNSET\" in a) echo a > o;; *) echo z > o;; esac",
        &shown,
    );
    assert_eq!(
        every.alternatives[0].texts.len(),
        2,
        "a `*` arm leaves no world where none matched"
    );
    let falls = run(
        "case \"$UNSET\" in a) echo a > o;& b) echo b > o;; esac",
        &shown,
    );
    assert!(falls.alternatives.is_empty() && !falls.written.iter().any(|w| w.path == "/repo/o"));
}

/// Everything an undecided `if` leaves joins: what its arms print into a
/// `$( )`, and whether one of them may have ended the shell. An exit inside a
/// subshell ends only the subshell.
#[test]
fn an_undecided_if_joins_what_it_prints_and_whether_it_stopped() {
    let shown = known(&[("/repo/f", Some("x\n"))]);
    let printed = run(
        "v=\"$(if [ -r f ]; then echo a; else echo b; fi)\"; echo $v > o",
        &shown,
    );
    assert!(
        !printed.written.iter().any(|w| w.path == "/repo/o"),
        "{:?}",
        printed.written
    );

    let stopped = run(
        "if [ -r f ]; then for i in $(./list); do exit; done; fi; echo a > o",
        &shown,
    );
    assert!(
        !stopped.written.iter().any(|w| w.path == "/repo/o"),
        "{:?}",
        stopped.written
    );

    let contained = run("( for i in $(./list); do exit; done ); echo a > o", &shown);
    assert_eq!(
        contained.written,
        vec![written("/repo/o", "a\n")],
        "{:?}",
        contained.unfollowed
    );
}

/// A `$( )` has the value of what it printed, its trailing newlines removed;
/// unquoted, only where splitting and globbing change nothing.
#[test]
fn a_command_substitution_has_the_value_it_printed() {
    let shown = known(&[("/repo/f", Some("first\nsecond x\n"))]);
    for (command, text) in [
        ("x=$(echo hi); echo $x > o", "hi\n"),
        ("d=$(dirname /a/b/c.txt); echo \"$d\" > o", "/a/b\n"),
        ("echo \"$(head -1 f)\" > o", "first\n"),
        ("echo \"$(cat f)\" > o", "first\nsecond x\n"),
        ("n=$(grep -c x f); echo \"n=$n\" > o", "n=1\n"),
        ("echo \"$(echo a | tr a-z A-Z)\" > o", "A\n"),
        ("b=$(basename \"$(dirname /x/y/z)\"); echo $b > o", "y\n"),
    ] {
        let found = run(command, &shown);
        let file = found.written.iter().find(|w| w.path == "/repo/o");
        assert_eq!(
            file,
            Some(&written("/repo/o", text)),
            "{command}: {:?}",
            found.unfollowed
        );
    }
    for command in [
        "echo $(tail -1 f) > o",
        "x=$(./tool); echo $x > o",
        "echo \"$(date)\" > o",
    ] {
        let found = run(command, &shown);
        assert!(
            !found.written.iter().any(|w| w.path == "/repo/o"),
            "{command}: {:?}",
            found.written
        );
    }
}

/// A loop this does not follow forgets only the names it binds, so a write in
/// it still reaches the file a name bound before it names; and a write to a
/// path the text does not determine withdraws what came before. Found in
/// history: `: > "$out"` then a glob loop appending to "$out" was predicted
/// empty.
#[test]
fn a_loop_not_followed_still_writes_where_its_names_say() {
    let found = run(
        "out=o\n: > \"$out\"\nfor f in *.rs; do b=$f; echo \"mod $b;\" >> \"$out\"; done",
        &nothing_known(),
    );
    assert!(
        !found.written.iter().any(|w| w.path == "/repo/o"),
        "{:?}",
        found.written
    );
    assert!(
        found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/o"))
    );

    let unnamed = run("echo x > a; echo y > \"$UNKNOWN\"", &nothing_known());
    assert!(unnamed.written.is_empty(), "{:?}", unnamed.written);
}

/// A test this can decide steers `&&`, `||` and `if` exactly: from the text,
/// from sight, and from what the run itself wrote. One it cannot decide is
/// not assumed to succeed, since what it asks is whether it does.
#[test]
fn a_test_sight_decides_steers_the_list_and_the_if() {
    let shown = Shown {
        files: known(&[
            ("/repo/here", Some("x")),
            ("/repo/empty", Some("")),
            ("/repo/gone", None),
        ]),
        dirs: Dirs::from([("/repo/d".to_string(), Some(Vec::new()))]),
    };
    for (command, a) in [
        ("false || echo x > a", Some("x\n")),
        ("true || echo x > a", None),
        ("[ -f here ] && echo x > a", Some("x\n")),
        ("[ -f gone ] && echo x > a", None),
        ("[ -s empty ] || echo x > a", Some("x\n")),
        ("[ ! -e gone ] && echo x > a", Some("x\n")),
        ("[ -d d ] && echo x > a", Some("x\n")),
        ("test -z '' && echo x > a", Some("x\n")),
        ("[[ -f here && ! -f gone ]] && echo x > a", Some("x\n")),
        ("v=b; [ \"$v\" = b ] && echo x > a", Some("x\n")),
        (
            "if [ -f gone ]; then echo y > a; else echo x > a; fi",
            Some("x\n"),
        ),
        ("if [ -f here ]; then echo x > a; fi", Some("x\n")),
        ("echo n > gone; [ -f gone ] && echo x > a", Some("x\n")),
        ("[ 3 -gt 2 ] && echo x > a", Some("x\n")),
        ("if grep -q x here; then echo x > a; fi", Some("x\n")),
        ("grep -q nothing here && echo x > a", None),
    ] {
        let parsed = reader::syntax::parse(command).expect("parses");
        let found = predict(&parsed, CWD, HOME, &shown);
        let file = found.written.iter().find(|w| w.path == "/repo/a");
        assert_eq!(
            file,
            a.map(|a| written("/repo/a", a)).as_ref(),
            "{command}: {:?}",
            found.unfollowed
        );
        assert!(
            !found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/a")),
            "{command}"
        );
    }
    for command in [
        "[ -f unshown ] && echo x > a",
        "[ -r here ] && echo x > a",
        "if grep -q x unshown; then echo x > a; fi",
        "! make && echo x > a",
    ] {
        let parsed = reader::syntax::parse(command).expect("parses");
        let found = predict(&parsed, CWD, HOME, &shown);
        assert!(
            !found.written.iter().any(|w| w.path == "/repo/a"),
            "{command}: {:?}",
            found.written
        );
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/a")),
            "{command}"
        );
    }
}

#[test]
fn a_write_after_or_is_only_sometimes_made() {
    let found = run("make || echo x > a", &nothing_known());
    assert!(found.written.is_empty());
    assert_eq!(found.unfollowed.len(), 1);
}

#[test]
fn a_quoted_loop_variable_is_the_word_it_is_bound_to() {
    let found = run(
        "for f in a b; do echo x > \"$f\"; done; echo y > c",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/a", "x\n"),
            written("/repo/b", "x\n"),
            written("/repo/c", "y\n")
        ]
    );
    assert!(found.unfollowed.is_empty());
}

#[test]
fn a_command_that_writes_nothing_predicts_nothing() {
    let found = run("ls -la && git status", &nothing_known());
    assert!(found.written.is_empty());
    assert!(found.unfollowed.is_empty());
}

mod checking {
    use super::*;
    use reader::predict::{Divergence, check};

    #[test]
    fn a_file_holding_what_was_predicted_agrees() {
        let after = known(&[("/repo/a", Some("x\n"))]);
        assert!(check(&[written("/repo/a", "x\n")], &after).is_empty());
    }

    #[test]
    fn a_file_holding_something_else_diverges() {
        let after = known(&[("/repo/a", Some("y\n"))]);
        assert_eq!(
            check(&[written("/repo/a", "x\n")], &after),
            vec![Divergence {
                path: "/repo/a".to_string(),
                predicted: Some("x\n".to_string()),
                actual: Some("y\n".to_string()),
            }]
        );
    }

    #[test]
    fn a_file_that_was_never_made_diverges() {
        let after = known(&[("/repo/a", None)]);
        assert_eq!(check(&[written("/repo/a", "x\n")], &after).len(), 1);
    }
}

/// A program that writes a file without a redirect is refused by name, with the
/// file it writes: a write the evaluator never mentions is neither a prediction
/// nor a refusal.
#[test]
fn a_program_that_writes_a_file_is_refused_with_its_path() {
    let found = run("sed -i 's/a/b/' notes.txt", &nothing_known());
    assert!(found.written.is_empty());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/notes.txt".to_string()),
            why: Why::NotRead,
        }]
    );
}

/// What a program wrote is unknown from then on, so a later read of it predicts
/// nothing. The control is the same read with no program between.
#[test]
fn a_file_a_program_wrote_is_forgotten() {
    let copied = run(
        "echo x > a.txt && cp src.txt a.txt && cat a.txt > b.txt",
        &nothing_known(),
    );
    assert!(copied.written.iter().all(|w| w.path != "/repo/b.txt"));
    assert!(copied.unfollowed.contains(&Unfollowed {
        path: Some("/repo/b.txt".to_string()),
        why: Why::NotRead,
    }));

    let plain = run("echo x > a.txt && cat a.txt > b.txt", &nothing_known());
    assert!(plain.written.contains(&written("/repo/b.txt", "x\n")));
}

#[test]
fn a_loop_over_words_the_text_does_not_spell_out_is_refused_as_a_compound() {
    let found = run("for f in $list; do rm gone.txt; done", &nothing_known());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/gone.txt".to_string()),
            why: Why::Compound,
        }]
    );
}

#[test]
fn a_loop_over_spelled_out_words_runs_once_per_word() {
    let found = run(
        "for f in a b; do echo $f > $f.txt; done; echo $f > last.txt",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/a.txt", "a\n"),
            written("/repo/b.txt", "b\n"),
            written("/repo/last.txt", "b\n"),
        ],
        "and the variable stays bound to the last word, as bash leaves it"
    );
    assert!(found.unfollowed.is_empty());
}

#[test]
fn a_group_is_its_commands_and_a_subshell_keeps_its_cd_inside() {
    let found = run(
        "{ echo x > a.txt; }; (cd sub && echo y > b.txt); echo z > c.txt",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/a.txt", "x\n"),
            written("/repo/sub/b.txt", "y\n"),
            written("/repo/c.txt", "z\n"),
        ]
    );
}

#[test]
fn a_variable_is_known_while_the_text_bound_it() {
    let found = run("name=out; echo hi > $name.txt", &nothing_known());
    assert_eq!(found.written, vec![written("/repo/out.txt", "hi\n")]);

    let rebound = run("name=out; read name; echo hi > $name.txt", &nothing_known());
    assert_eq!(
        rebound.unfollowed,
        vec![Unfollowed {
            path: None,
            why: Why::Expansion,
        }],
        "a builtin that binds on its own leaves the name unknown"
    );

    let computed = run("name=$(date); echo hi > $name.txt", &nothing_known());
    assert!(computed.written.is_empty());

    let prefix = run("name=out true; echo hi > $name.txt", &nothing_known());
    assert!(
        prefix.written.is_empty(),
        "a prefix binds for that command alone"
    );

    let inner = run("(name=in); echo hi > ${name:-none}.txt", &nothing_known());
    assert!(inner.written.is_empty(), "an operator is not followed");
    let subshell = run("name=out; (name=in); echo hi > $name.txt", &nothing_known());
    assert_eq!(subshell.written, vec![written("/repo/out.txt", "hi\n")]);
}

/// `sed -i` over a file sight has shown, with the old text kept under `-i.bak`.
#[test]
fn sed_in_place_rewrites_the_file_sight_has_shown() {
    let shown = known(&[("/repo/a.txt", Some("x y\nx\n"))]);
    let found = run("sed -i '' 's/x/z/' a.txt", &shown);
    assert_eq!(found.written, vec![written("/repo/a.txt", "z y\nz\n")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);

    let backup = run("sed -i.bak -e 's/x/z/g' -e '$s/y/w/' a.txt", &shown);
    assert_eq!(
        backup.written,
        vec![
            written("/repo/a.txt.bak", "x y\nx\n"),
            written("/repo/a.txt", "z y\nz\n"),
        ]
    );

    let blind = run("sed -i '' 's/x/z/' a.txt", &nothing_known());
    assert_eq!(
        blind.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/a.txt".to_string()),
            why: Why::NotRead,
        }]
    );

    let deletes = run("sed -i '' '2d' a.txt", &shown);
    assert_eq!(deletes.written, vec![written("/repo/a.txt", "x y\n")]);
    let translates = run("sed -i '' 'y/x/z/' a.txt", &shown);
    assert!(translates.written.is_empty());
    assert_eq!(
        translates.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/a.txt".to_string()),
            why: Why::Sed("y".to_string()),
        }]
    );

    let quiet = run("sed -n -i '' 's/x/z/p' a.txt", &shown);
    assert_eq!(quiet.unfollowed[0].why, Why::Sed("-n".to_string()));
}

/// Found live: a heredoc edit whose output went to `grep` was refused whole.
/// Each member runs in a subshell of its own.
#[test]
fn each_member_of_a_pipeline_is_followed_in_its_own_subshell() {
    let found = run(
        "python3 - <<'PY' 2>&1 | head -3\nopen('a.txt', 'w').write('x')\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/a.txt", "x")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);

    let scoped = run(
        "(cd sub && echo y > b.txt) | cat; echo z > c.txt",
        &nothing_known(),
    );
    assert_eq!(
        scoped.written,
        vec![
            written("/repo/sub/b.txt", "y\n"),
            written("/repo/c.txt", "z\n")
        ]
    );

    // A member reading what the one before printed, and a path two members
    // both change.
    let piped = run("echo x | cat > d.txt", &nothing_known());
    assert_eq!(piped.written, vec![written("/repo/d.txt", "x\n")]);
    let unread = run("./tool | cat > d.txt", &nothing_known());
    assert!(unread.written.is_empty());
    assert_eq!(
        unread.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/d.txt".to_string()),
            why: Why::Program("tool".to_string()),
        }]
    );
    let raced = run("echo x > e.txt | echo y > e.txt", &nothing_known());
    assert!(raced.written.is_empty());
    assert_eq!(raced.unfollowed.last().unwrap().why, Why::Pipeline);

    // A program's output in a pipeline is refused for the program, as alone.
    let output = run("echo a | awk '{print}' > g.txt", &nothing_known());
    assert_eq!(
        output.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/g.txt".to_string()),
            why: Why::Program("awk".to_string()),
        }]
    );
}

/// Found in history: `for pair in a:b …; do s=${pair%%:*}; g=${pair##*:}; … > $g.svg`.
#[test]
fn a_bound_name_under_a_strip_operator_is_computed() {
    let found = run(
        "pair=20260717:tables; s=${pair%%:*}; g=${pair##*:}; echo x > $s-$g.txt; echo y > \"$HOME/h.txt\"; echo z > $PWD/p.txt",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/20260717-tables.txt", "x\n"),
            written("/home/me/h.txt", "y\n"),
            written("/repo/p.txt", "z\n"),
        ]
    );

    let ends = run(
        "f=a.b.c; echo x > ${f%.*}.txt; echo y > ${f#*.}.txt; echo z > ${f%%.*}.txt; echo w > ${f##*.}.txt; echo v > ${f%x*}.txt",
        &nothing_known(),
    );
    assert_eq!(
        ends.written,
        vec![
            written("/repo/a.b.txt", "x\n"),
            written("/repo/b.c.txt", "y\n"),
            written("/repo/a.txt", "z\n"),
            written("/repo/c.txt", "w\n"),
            written("/repo/a.b.c.txt", "v\n"),
        ]
    );

    let unknown = run(
        "echo x > ${undefined%%:*}.txt; echo y > ${f:-d}.txt",
        &nothing_known(),
    );
    assert!(unknown.written.is_empty());
    assert!(unknown.unfollowed.iter().all(|u| u.why == Why::Expansion));
}

/// `mv` of one file to one file: the destination holds the source's text and
/// the source is gone.
#[test]
fn mv_of_a_file_sight_has_shown_moves_its_text() {
    let shown = known(&[("/repo/a.txt", Some("x\n")), ("/repo/b.txt", None)]);
    let found = run("mv a.txt b.txt", &shown);
    assert_eq!(
        found.written,
        vec![written("/repo/b.txt", "x\n"), removed("/repo/a.txt")]
    );
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);

    let then = run("mv a.txt b.txt && cat a.txt >> b.txt", &shown);
    assert_eq!(
        then.unfollowed[0].why,
        Why::Missing,
        "the source is gone: {:?}",
        then.unfollowed
    );

    let blind = run("mv a.txt b.txt", &nothing_known());
    assert_eq!(
        blind.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/b.txt".to_string()),
            why: Why::NotRead,
        }]
    );
    assert!(blind.written.is_empty());

    let clobber = run("mv -n a.txt b.txt", &shown);
    assert_eq!(clobber.unfollowed[0].why, Why::Option("mv -n".to_string()));
    let many = run("mv a.txt b.txt dir.d", &shown);
    assert_eq!(
        many.unfollowed[0].why,
        Why::Option("mv into a directory".to_string())
    );
}

/// Found in review: an unrolled loop ran every iteration past a `break`, and a
/// script went on past `exit`.
#[test]
fn control_flow_is_honoured_or_refused() {
    let looped = run(
        "for i in 1 2 3; do echo x > log$i.txt; if ok; then break; fi; done",
        &nothing_known(),
    );
    assert!(looped.written.is_empty(), "{:?}", looped.written);
    assert!(looped.unfollowed.iter().all(|u| u.why == Why::Compound));

    let exited = run("echo x > a.txt; exit 0; echo y > b.txt", &nothing_known());
    assert_eq!(exited.written, vec![written("/repo/a.txt", "x\n")]);
    assert!(
        exited.unfollowed.is_empty(),
        "what never runs is not a write"
    );

    let inner = run(
        "(echo x > a.txt; exit 1); (exit 1) | cat; bash -c 'exit 1'; echo y > b.txt",
        &nothing_known(),
    );
    assert_eq!(
        inner.written,
        vec![written("/repo/a.txt", "x\n"), written("/repo/b.txt", "y\n")],
        "an exit ends only the shell it is in"
    );
}

/// Found in review: a binding survived a compound this does not follow, and a
/// child shell saw its parent's bindings.
#[test]
fn a_binding_is_unknown_after_anything_that_could_rebind_it() {
    for script in [
        "x=a; if c; then x=b; fi; echo y > $x.txt",
        "x=a; ((x++)); echo y > $x.txt",
        "x=a; make || x=b; echo y > $x.txt",
        "x=a; printf -v x b; echo y > $x.txt",
        "x=a; bash -c 'echo y > $x.txt'",
    ] {
        let found = run(script, &nothing_known());
        assert!(found.written.is_empty(), "{script}: {:?}", found.written);
    }
    let kept = run(
        "x=a; make || echo; bash -c 'x=b'; echo y > $x.txt",
        &nothing_known(),
    );
    assert_eq!(kept.written, vec![written("/repo/a.txt", "y\n")]);
}

/// Found in history: the loops refused as `python for` were over lists the
/// program could name — a range, a shown file's lines, a split string.
/// A list changed in place is followed where the change is modelled: an item,
/// a slice, and the methods that add or reorder. Found live: an insertion by
/// `lines[i+1:i+1] = ins` into thirteen `flake.nix` files was predicted as no
/// change at all.
#[test]
fn a_python_list_changed_in_place_is_followed() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[("/repo/a", Some("p\nq"))]);
    let head = "lines = open('a').read().split('\\n')\n";
    let tail = "\nopen('a', 'w').write('\\n'.join(lines))";
    for (change, after) in [
        ("lines[1:1] = ['x']", "p\nx\nq"),
        ("lines[0] = 'P'", "P\nq"),
        ("lines[-1:] = []", "p"),
        ("lines.append('z')", "p\nq\nz"),
        ("lines.insert(1, 'x')", "p\nx\nq"),
        ("lines.extend(['y', 'z'])", "p\nq\ny\nz"),
        ("lines.reverse()", "q\np"),
        ("del lines[0]", "q"),
        ("lines += ['z']", "p\nq\nz"),
        ("n = [l for l in lines]", "p\nq"),
        ("del lines[0], lines[0]", ""),
    ] {
        let found = run(&py(&format!("{head}{change}{tail}")), &shown);
        assert_eq!(found.written, vec![written("/repo/a", after)], "{change}");
    }
}

/// A comprehension runs as Python runs it: in order, its variables its own,
/// and a generator only as far as what consumes it looks. Found live: the
/// insertion point of `lines[i:i] = ins` came from `next(... for ...)`.
#[test]
fn a_python_comprehension_is_followed() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let head = "import json\nlines = ['p', 'q']\n";
    for (program, text) in [
        (
            "out = [l + '!' for l in lines if l != 'q']\nw('/'.join(out))",
            "p!",
        ),
        (
            "w(','.join([a + b for a in 'xy' for b in '12']))",
            "x1,x2,y1,y2",
        ),
        ("l = 'keep'\nn = [l for l in lines]\nw(l)", "keep"),
        (
            "w(json.dumps({k: v for k, v in [('a', '1'), ('b', '2')]}))",
            "{\"a\": \"1\", \"b\": \"2\"}",
        ),
        ("w(''.join(c for c in 'abc' if c != 'b'))", "ac"),
        (
            "i = next(n for n, l in enumerate(lines) if l == 'q')\nlines[i:i] = ['x']\nw(''.join(lines))",
            "pxq",
        ),
        // Lazy: the second element would raise, and is never reached.
        (
            "w(next(l for l in lines if l == 'p' or lines[5] == 'z'))",
            "p",
        ),
        ("w(next((l for l in lines if l == 'z'), 'none'))", "none"),
        (
            "w(str(any(l == 'q' for l in lines)) + str(all(l == 'q' for l in lines)))",
            "TrueFalse",
        ),
    ] {
        let body = format!("{head}def w(t):\n    open('o', 'w').write(t)\n{program}");
        let found = run(&py(&body), &nothing_known());
        assert_eq!(
            found.written,
            vec![written("/repo/o", text)],
            "{program}: {:?}",
            found.unfollowed
        );
    }
}

/// A directory's names come in no order, so a listing is a set: `sorted` of
/// it is exact, a loop over it is not. What the run wrote there is in it, and
/// after a directory is made untracked no listing is known.
#[test]
fn a_python_directory_listing_is_a_set_of_names() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = Shown {
        files: known(&[("/repo/src/a.rs", Some("A")), ("/repo/src/b.rs", Some("B"))]),
        dirs: Dirs::from([
            (
                "/repo".to_string(),
                Some(vec![
                    "b.txt".to_string(),
                    "a.txt".to_string(),
                    ".h.txt".to_string(),
                    "c.md".to_string(),
                ]),
            ),
            (
                "/repo/src".to_string(),
                Some(vec![
                    "b.rs".to_string(),
                    "a.rs".to_string(),
                    ".x.rs".to_string(),
                ]),
            ),
        ]),
    };
    let head =
        "import glob, os\nfrom pathlib import Path\ndef w(t):\n    open('o', 'w').write(t)\n";
    for (program, text) in [
        ("w(','.join(sorted(glob.glob('*.txt'))))", "a.txt,b.txt"),
        (
            "open('d.txt', 'w').write('')\nw(','.join(sorted(glob.glob('*.txt'))))",
            "a.txt,b.txt,d.txt",
        ),
        ("w(','.join(sorted(os.listdir('src'))))", ".x.rs,a.rs,b.rs"),
        (
            "w(','.join(str(p) for p in sorted(Path('src').glob('*.rs'))))",
            "src/.x.rs,src/a.rs,src/b.rs",
        ),
        (
            "w(str(len(glob.glob('src/?.rs'))) + str('src/a.rs' in glob.glob('src/*')))",
            "2True",
        ),
    ] {
        let parsed = reader::syntax::parse(&py(&format!("{head}{program}"))).expect("parses");
        let found = predict(&parsed, "/repo", "/home/me", &shown);
        let file = found.written.iter().find(|w| w.path == "/repo/o");
        assert_eq!(
            file,
            Some(&written("/repo/o", text)),
            "{program}: {:?}",
            found.unfollowed
        );
    }
    let edited = reader::syntax::parse(&py(
        "import glob\nfor p in sorted(glob.glob('src/*.rs')):\n    s = open(p).read()\n    open(p, 'w').write(s + '!')",
    ))
    .expect("parses");
    assert_eq!(
        predict(&edited, "/repo", "/home/me", &shown).written,
        vec![
            written("/repo/src/a.rs", "A!"),
            written("/repo/src/b.rs", "B!")
        ]
    );
    for program in [
        "for p in glob.glob('*.txt'):\n    w(p)",
        "os.makedirs('x')\nw(','.join(sorted(os.listdir('.'))))",
        "w(','.join(sorted(os.listdir('elsewhere'))))",
    ] {
        let parsed = reader::syntax::parse(&py(&format!("{head}{program}"))).expect("parses");
        let found = predict(&parsed, "/repo", "/home/me", &shown);
        assert!(
            !found.written.iter().any(|w| w.path == "/repo/o"),
            "{program}: {:?}",
            found.written
        );
    }
}

/// A match is followed as Python's `re` gives it: groups by number and name,
/// positions in characters, `finditer`, `findall`, and `re.sub` calling a
/// function or a lambda once per match.
#[test]
fn a_python_regex_match_is_followed() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let head = "import re, json\ndef w(t):\n    open('o', 'w').write(t)\n";
    for (program, text) in [
        (
            "m = re.search(r'v(\\d+)', 'app v12 x')\nw(m.group(1) + str(m.start()) + m[0])",
            "124v12",
        ),
        (
            "w(str(re.match('x', 'ax')) + str(re.fullmatch('a.', 'ax').end()))",
            "None2",
        ),
        (
            "m = re.search(r'(?P<k>\\w+)=(?P<v>\\w+)', 'a=1')\nw(m.group('v') + json.dumps(m.groupdict()))",
            "1{\"k\": \"a\", \"v\": \"1\"}",
        ),
        (
            "w(''.join([m.group() for m in re.finditer(r'\\d', 'a1b2')]))",
            "12",
        ),
        (
            "w(json.dumps(re.findall(r'(\\w)=(\\d)', 'a=1 b=2')) + json.dumps(re.findall(r'\\d', 'a1b2')))",
            "[[\"a\", \"1\"], [\"b\", \"2\"]][\"1\", \"2\"]",
        ),
        (
            "w(re.sub(r'\\d', lambda m: '<' + m.group() + '>', 'a1b2'))",
            "a<1>b<2>",
        ),
        (
            "def br(m):\n    return '[' + m.group(1) + ']'\nw(re.sub(r'(\\d)', br, 'a1b2', count=1))",
            "a[1]b2",
        ),
        (
            "p = re.compile(r'b')\nw(str(p.search('abc').start()) + str(re.search('é', 'aé').start()))",
            "11",
        ),
    ] {
        let found = run(&py(&format!("{head}{program}")), &nothing_known());
        assert_eq!(
            found.written,
            vec![written("/repo/o", text)],
            "{program}: {:?}",
            found.unfollowed
        );
    }
}

/// A set is followed where its order does not matter, and refused where it
/// does: Python gives it none.
#[test]
fn a_python_set_is_followed_where_order_does_not_matter() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let w = "def w(t):\n    open('o', 'w').write(t)\n";
    for (program, text) in [
        (
            "s = set(['b', 'a', 'b'])\ns.add('c')\ns.discard('a')\nw(','.join(sorted(s)) + str(len(s)))",
            "b,c2",
        ),
        (
            "s = {x for x in 'abca'} | {'z'}\nw(','.join(sorted(s)) + str('z' in s))",
            "a,b,c,zTrue",
        ),
        ("s = {'a', 'b'} - {'b'}\nw(','.join(sorted(s)))", "a"),
    ] {
        let found = run(&py(&format!("{w}{program}")), &nothing_known());
        assert_eq!(
            found.written,
            vec![written("/repo/o", text)],
            "{program}: {:?}",
            found.unfollowed
        );
    }
    for program in [
        "for x in {'a', 'b'}:\n    w(x)",
        "w(','.join(list({'a', 'b'})))",
    ] {
        let found = run(&py(&format!("{w}{program}")), &nothing_known());
        assert!(found.written.is_empty(), "{program}: {:?}", found.written);
    }
}

/// Python shares a list between everything that holds it; this copies, so a
/// change it cannot see through every holder forgets the list rather than
/// write its old text.
#[test]
fn a_python_list_changed_where_this_cannot_follow_is_forgotten() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[("/repo/a", Some("p\nq"))]);
    let head = "lines = open('a').read().split('\\n')\n";
    let tail = "\nopen('a', 'w').write('\\n'.join(lines))";
    for change in [
        "other = lines\nother.append('z')",
        "rows = [lines]\nrows[0].append('z')",
        "d = {'k': lines}\nd['k'].append('z')",
        "def grow(xs):\n    xs.append('z')\ngrow(lines)",
        "import random\nrandom.shuffle(lines)",
        "lines.sort()",
        "lines.pop()",
        "if len(open('b').read()):\n    lines.append('z')",
        // Found live: trailing lines popped in a loop, predicted still there.
        "while lines and lines[-1] == 'q':\n    lines.pop()",
        "i = next(n for n, l in enumerate(lines) if f(l))\nlines[i:i] = ['x']",
        "import os\na, lines[0] = os.environ.get('X')",
    ] {
        let found = run(&py(&format!("{head}{change}{tail}")), &shown);
        assert!(found.written.is_empty(), "{change}: {:?}", found.written);
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/a")),
            "{change}: {:?}",
            found.unfollowed
        );
    }
}

/// A module-level lambda or function that changes the module's list changes
/// it where it lives, and the write after it is followed. Kept apart from the
/// forgotten shapes above because one of these — the lambda — sat among them
/// until the change was written back where the name is read from
/// (2026-10-05); forgetting it was never wrong, only less than the text says.
#[test]
fn a_python_list_changed_by_a_module_level_callable_is_followed() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[("/repo/a", Some("p\nq"))]);
    let head = "lines = open('a').read().split('\\n')\n";
    let tail = "\nopen('a', 'w').write('\\n'.join(lines))";
    for change in [
        "grow = lambda: lines.append('z')\ngrow()",
        "def grow():\n    lines.append('z')\ngrow()",
        "def put(i, v):\n    lines[i] = v\nput(1, 'z')\nlines.append('z')",
    ] {
        let found = run(&py(&format!("{head}{change}{tail}")), &shown);
        assert_eq!(
            found.written,
            vec![written(
                "/repo/a",
                if change.starts_with("def put") {
                    "p\nz\nz"
                } else {
                    "p\nq\nz"
                }
            )],
            "{change}: {:?}",
            found.unfollowed
        );
    }
}

#[test]
fn a_python_loop_over_what_the_program_can_list_is_unrolled() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[("/repo/a.txt", Some("p\nq\n"))]);

    let ranged = run(
        &py("for i in range(2):\n    open(str(i) + '.txt', 'w').write('x')"),
        &shown,
    );
    assert_eq!(
        ranged.written,
        vec![written("/repo/0.txt", "x"), written("/repo/1.txt", "x")]
    );

    let lines = run(
        &py("for line in open('a.txt'):\n    open(line.strip() + '.out', 'w').write(line)"),
        &shown,
    );
    assert_eq!(
        lines.written,
        vec![written("/repo/p.out", "p\n"), written("/repo/q.out", "q\n")]
    );
    let read = run(
        &py(
            "for line in open('a.txt').read().splitlines():\n    open(line + '.o', 'w').write('y')",
        ),
        &shown,
    );
    assert_eq!(
        read.written,
        vec![written("/repo/p.o", "y"), written("/repo/q.o", "y")]
    );
    let readlines = run(
        &py("for line in open('a.txt').readlines():\n    open(line.strip(), 'w').write('z')"),
        &shown,
    );
    assert_eq!(
        readlines.written,
        vec![written("/repo/p", "z"), written("/repo/q", "z")]
    );

    let split = run(
        &py("for name in 'u.txt v.txt'.split():\n    open(name, 'w').write('x')"),
        &shown,
    );
    assert_eq!(
        split.written,
        vec![written("/repo/u.txt", "x"), written("/repo/v.txt", "x")]
    );
    let enumerated = run(
        &py("for i, name in enumerate(['u', 'v'], 1):\n    open(name + str(i), 'w').write('x')"),
        &shown,
    );
    assert_eq!(
        enumerated.written,
        vec![written("/repo/u1", "x"), written("/repo/v2", "x")]
    );
    let ordered = run(
        &py("for name in sorted(['b.txt', 'a.txt']):\n    open(name, 'w').write('x')"),
        &shown,
    );
    assert_eq!(
        ordered.written,
        vec![written("/repo/a.txt", "x"), written("/repo/b.txt", "x")]
    );
    let zipped = run(
        &py("for a, b in zip(['m', 'n'], ['1', '2']):\n    open(a + b, 'w').write('x')"),
        &shown,
    );
    assert_eq!(
        zipped.written,
        vec![written("/repo/m1", "x"), written("/repo/n2", "x")]
    );
    let joined = run(&py("open('-'.join(['j', 'k']), 'w').write('x')"), &shown);
    assert_eq!(joined.written, vec![written("/repo/j-k", "x")]);

    let unknown = run(
        &py("for name in sorted(names()):\n    open(name, 'w').write('x')"),
        &shown,
    );
    assert!(unknown.written.is_empty());
    assert!(
        unknown
            .unfollowed
            .iter()
            .any(|u| u.why == Why::Python("for over call names".to_string())),
        "{:?}",
        unknown.unfollowed
    );
}

/// `cp` of one file to one file: the destination holds the source's text.
#[test]
fn cp_of_a_file_sight_has_shown_is_its_text() {
    let shown = known(&[("/repo/a.txt", Some("x\n")), ("/repo/b.txt", None)]);
    let found = run("cp a.txt b.txt", &shown);
    assert_eq!(found.written, vec![written("/repo/b.txt", "x\n")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);

    let then = run("cp a.txt b.txt && echo y >> b.txt", &shown);
    assert_eq!(then.written, vec![written("/repo/b.txt", "x\ny\n")]);

    // From history nothing is shown, and the write depends on a text nobody has.
    let blind = run("cp a.txt b.txt", &nothing_known());
    assert!(blind.written.is_empty());
    assert_eq!(
        blind.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/b.txt".to_string()),
            why: Why::NotRead,
        }]
    );

    // A destination sight cannot show may be a directory, which would make
    // the written file a different one.
    let unseen = run("cp a.txt out.d", &known(&[("/repo/a.txt", Some("x\n"))]));
    assert_eq!(unseen.unfollowed[0].why, Why::NotRead);

    // A source that does not exist fails the copy.
    let gone = run(
        "cp a.txt b.txt",
        &known(&[("/repo/a.txt", None), ("/repo/b.txt", None)]),
    );
    assert_eq!(gone.unfollowed[0].why, Why::Missing);

    // A tree, and several sources into a directory, are refused by name.
    let tree = run("cp -r src.d dst.d", &shown);
    assert_eq!(tree.unfollowed[0].why, Why::Option("cp -r".to_string()));
    let many = run("cp a.txt b.txt dir.d", &shown);
    assert_eq!(
        many.unfollowed[0].why,
        Why::Option("cp into a directory".to_string())
    );
}

/// A destination the text names is still named when a source is a variable.
#[test]
fn a_named_destination_survives_an_expanded_source() {
    let found = run("cp \"$src\" dest.txt", &nothing_known());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/dest.txt".to_string()),
            why: Why::Program("cp".to_string()),
        }]
    );
}

/// The corpus's commonest Python edit: read, replace, write back. Given the
/// file's text it is predicted; the file is what `needs` asks for.
#[test]
fn a_python_edit_is_predicted_from_the_file_it_reads() {
    let script = "python3 - <<'PY'\np = 'a.txt'\ns = open(p).read()\ns = s.replace('x', 'y', 1)\nopen(p, 'w').write(s)\nPY";
    let parsed = reader::syntax::parse(script).expect("parses");
    assert_eq!(needs(&parsed, CWD, HOME), vec!["/repo/a.txt".to_string()]);
    let found = run(script, &known(&[("/repo/a.txt", Some("x x\n"))]));
    assert_eq!(found.written, vec![written("/repo/a.txt", "y x\n")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);
}

/// From history the file is not given, so the write depends on text nobody
/// has: refused, with the file named.
#[test]
fn a_python_edit_from_history_is_not_read() {
    let found = run(
        "python3 - <<'PY'\ns = open('a.txt').read()\nopen('a.txt', 'w').write(s + 'z')\nPY",
        &nothing_known(),
    );
    assert!(found.written.is_empty());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/a.txt".to_string()),
            why: Why::NotRead,
        }]
    );
}

/// Found live: a loop over `grep -rl`'s output edited three files, and a later
/// loop over those files by name was predicted from their text as if the first
/// had not run. A write to a path the program cannot name may have been to any
/// file, so nothing read after it is known.
#[test]
fn a_write_to_a_path_the_program_cannot_name_leaves_every_later_read_unknown() {
    let found = run(
        "python3 - <<'PY'\nfor p in names():\n    open(p, 'w').write('x')\ns = open('a.txt').read()\nopen('a.txt', 'w').write(s + 'z')\nPY",
        &known(&[("/repo/a.txt", Some("a\n"))]),
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(
        found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/a.txt")),
        "the later edit is refused by name: {:?}",
        found.unfollowed
    );
}

#[test]
fn pathlib_reads_and_writes_are_followed() {
    let found = run(
        "python3 - <<'PY'\nfrom pathlib import Path\np = Path('src') / 'm.rs'\np.write_text(p.read_text().replace('a', 'b'))\nPY",
        &known(&[("/repo/src/m.rs", Some("aa\n"))]),
    );
    assert_eq!(found.written, vec![written("/repo/src/m.rs", "bb\n")]);
}

/// A failed `assert` raises, and nothing after it runs. The control is the same
/// program with the assertion holding.
#[test]
fn a_failing_assert_ends_the_program() {
    let program = |needle: &str| {
        format!(
            "python3 - <<'PY'\ns = open('a.txt').read()\nassert '{needle}' in s\nopen('a.txt', 'w').write('new')\nPY"
        )
    };
    let given = known(&[("/repo/a.txt", Some("old\n"))]);
    let failed = run(&program("zz"), &given);
    assert!(failed.written.is_empty());
    assert!(failed.unfollowed.is_empty(), "{:?}", failed.unfollowed);

    let held = run(&program("old"), &given);
    assert_eq!(held.written, vec![written("/repo/a.txt", "new")]);
}

#[test]
fn writes_through_a_with_block_accumulate() {
    let found = run(
        "python3 -c \"with open('o.txt', 'w') as f:\n    f.write('a')\n    f.write('b')\"",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o.txt", "ab")]);
}

/// A shell command after the program sees what the program wrote.
#[test]
fn the_shell_reads_what_python_wrote() {
    let found = run(
        "python3 -c \"open('a.txt', 'w').write('hi\\n')\" && cat a.txt > b.txt",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/a.txt", "hi\n"),
            written("/repo/b.txt", "hi\n")
        ]
    );
}

/// A write under a condition the text does not decide is refused and the file
/// forgotten, the way a shell `if` is.
#[test]
fn a_write_under_an_undecided_if_is_refused() {
    let found = run(
        "python3 - <<'PY'\nimport os\nif os.environ.get('X'):\n    open('b.txt', 'w').write('x')\nPY",
        &nothing_known(),
    );
    assert!(found.written.is_empty());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/b.txt".to_string()),
            why: Why::Python("if on call os.environ.get".to_string()),
        }]
    );
}

#[test]
fn a_deletion_is_refused_by_the_call_that_made_it() {
    let found = run(
        "python3 -c \"import os; os.remove('a.txt')\"",
        &nothing_known(),
    );
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/a.txt".to_string()),
            why: Why::Python("os.remove".to_string()),
        }]
    );
}

/// Python does not expand `~`; `expanduser` does.
#[test]
fn a_home_path_needs_expanduser() {
    let expanded = run(
        "python3 -c \"import os; open(os.path.expanduser('~/x.txt'), 'w').write('a')\"",
        &nothing_known(),
    );
    assert_eq!(expanded.written, vec![written("/home/me/x.txt", "a")]);

    let literal = run(
        "python3 -c \"open('~/x.txt', 'w').write('a')\"",
        &nothing_known(),
    );
    assert!(literal.written.is_empty());
}

/// A library object saving itself names the file it writes, whatever the object.
#[test]
fn an_object_saved_to_a_path_writes_that_path() {
    let found = run(
        "python3 -c \"from PIL import Image; Image.open('a.png').crop((0,0,9,9)).save('b.png')\"",
        &nothing_known(),
    );
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/b.png".to_string()),
            why: Why::Python("method save".to_string()),
        }]
    );
}

/// A script handed to another shell is the same language, run against the same
/// files, and followed the same way.
#[test]
fn a_nested_shell_is_followed() {
    let found = run(
        "nix develop -c bash -c 'echo hi > a.txt' && cat a.txt > b.txt",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/a.txt", "hi\n"),
            written("/repo/b.txt", "hi\n")
        ]
    );
}

/// A program inside the nested shell rewrites a file the outer one wrote, and
/// the rewrite is followed against that text.
#[test]
fn a_rewrite_inside_a_nested_shell_is_followed_against_the_outer_write() {
    let found = run(
        "echo x > a.txt && bash -c 'sed -i s/x/y/ a.txt'",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/a.txt", "y\n")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);
}

/// The inner shell is a child: its `cd` does not move the outer one.
#[test]
fn a_nested_cd_stays_inside() {
    let found = run("bash -c 'cd sub' && echo x > a.txt", &nothing_known());
    assert_eq!(found.written, vec![written("/repo/a.txt", "x\n")]);
}

/// A script whose text the outer shell expands first is not known; every file
/// it writes is refused.
#[test]
fn an_expanded_nested_script_is_refused() {
    let found = run("bash -c \"echo $X > a.txt\"", &nothing_known());
    assert!(found.written.is_empty());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/a.txt".to_string()),
            why: Why::Expansion,
        }]
    );
}

/// A formatter over a directory makes every file under it unknown: the ones the
/// command wrote and the ones it was given.
#[test]
fn a_rewrite_of_a_directory_forgets_the_files_under_it() {
    let found = run(
        "cat > src/a.py <<'EOF'\nx=1\nEOF\nruff format src && cat src/b.py > c.txt",
        &known(&[("/repo/src/b.py", Some("y\n"))]),
    );
    assert!(found.written.iter().all(|w| w.path != "/repo/src/a.py"));
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/src/a.py".to_string()),
        why: Why::Program("ruff".to_string()),
    }));
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/c.txt".to_string()),
        why: Why::NotRead,
    }));
}

/// A path the command implies rather than names — `cargo fmt` rewrites where it
/// stands — is still determined by the text.
#[test]
fn an_implied_directory_is_named_when_the_text_determines_it() {
    let found = run(
        "echo x > src/m.rs && nix develop -c bash -c 'cargo fmt -p m'",
        &nothing_known(),
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/src/m.rs".to_string()),
        why: Why::Program("cargo".to_string()),
    }));
}

/// A `cd` inside a nested shell decides where its paths resolve, and is gone
/// with its subshell after; a pipeline member's `cd` likewise; one that only
/// sometimes ran leaves the directory unknown.
#[test]
fn a_cd_inside_a_forgotten_region_resolves_its_paths() {
    let found = run(
        "echo x > sub/a.ts && bash -c 'cd sub && prettier --write a.ts' | cat",
        &nothing_known(),
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/sub/a.ts".to_string()),
        why: Why::Program("prettier".to_string()),
    }));

    let piped = run("cd sub | cat; echo y > b.txt", &nothing_known());
    assert_eq!(piped.written, vec![written("/repo/b.txt", "y\n")]);

    let sometimes = run("make || cd sub; echo y > b.txt", &nothing_known());
    assert!(sometimes.written.is_empty(), "{:?}", sometimes.written);
}

/// A `Path` handed to a function this does not know may be written by it — a
/// helper brought in with `exec` restamped a file the heredoc had just appended
/// to. The control: the same function given a plain string predicts as before.
#[test]
fn a_path_given_to_an_unknown_function_is_forgotten() {
    let found = run(
        "echo x > a.md && python3 - <<'PY'\nfrom pathlib import Path\nexec(open('/tmp/lib.py').read())\nstamp(Path('a.md'))\nPY",
        &nothing_known(),
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/a.md".to_string()),
        why: Why::Python("call stamp".to_string()),
    }));

    let text = run(
        "echo x > a.md && python3 -c \"print('a.md')\"",
        &nothing_known(),
    );
    assert_eq!(text.written, vec![written("/repo/a.md", "x\n")]);
}

/// The corpus's helper: a function that edits a global string in place. Each
/// call is followed with its arguments.
#[test]
fn a_helper_function_editing_a_global_is_followed() {
    let program = "python3 - <<'PY'\np = 'a.txt'\ns = open(p).read()\ndef sub(old, new):\n    global s\n    assert old in s, old\n    s = s.replace(old, new, 1)\nsub('x', 'y')\nsub('b', 'c')\nopen(p, 'w').write(s)\nPY";
    let found = run(program, &known(&[("/repo/a.txt", Some("x b\n"))]));
    assert_eq!(found.written, vec![written("/repo/a.txt", "y c\n")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);

    // The same helper whose assertion fails ends the program before the write.
    let failed = run(program, &known(&[("/repo/a.txt", Some("q b\n"))]));
    assert!(failed.written.is_empty(), "{:?}", failed.written);
}

#[test]
fn a_function_with_locals_and_a_return_is_followed() {
    let found = run(
        "python3 - <<'PY'\nfrom pathlib import Path\ndef edit(path, old, new='r'):\n    p = Path(path)\n    t = p.read_text()\n    p.write_text(t.replace(old, new))\n    return len(t)\nn = edit('a.txt', 'q')\nPY",
        &known(&[("/repo/a.txt", Some("qq"))]),
    );
    assert_eq!(found.written, vec![written("/repo/a.txt", "rr")]);
}

/// A local stays in its frame: the outer `s` is written unchanged.
#[test]
fn a_functions_local_does_not_leak() {
    let found = run(
        "python3 - <<'PY'\ns = 'kept'\ndef f():\n    s = 'lost'\nf()\nopen('o.txt', 'w').write(s)\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o.txt", "kept")]);
}

#[test]
fn a_loop_over_a_written_out_list_runs_once_per_element() {
    let found = run(
        "python3 - <<'PY'\ns = open('a.txt').read()\nfor old, new in [('a', 'b'), ('c', 'd')]:\n    s = s.replace(old, new)\nopen('a.txt', 'w').write(s)\nPY",
        &known(&[("/repo/a.txt", Some("ac"))]),
    );
    assert_eq!(found.written, vec![written("/repo/a.txt", "bd")]);
}

#[test]
fn break_continue_and_else_in_a_loop() {
    let found = run(
        "python3 - <<'PY'\nout = ''\nfor x in ['a', 'skip', 'b', 'stop', 'c']:\n    if x == 'skip':\n        continue\n    if x == 'stop':\n        break\n    out += x\nelse:\n    out += 'never'\nopen('o.txt', 'w').write(out)\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o.txt", "ab")]);
}

#[test]
fn a_scripts_main_block_is_followed() {
    let found = run(
        "python3 - <<'PY'\nif __name__ == '__main__':\n    open('o.txt', 'w').write('main')\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o.txt", "main")]);
}

/// The corpus's splice: positions from `index`, a slice either side.
#[test]
fn a_splice_between_two_positions_is_followed() {
    let found = run(
        "python3 - <<'PY'\ns = open('a.txt').read()\ni = s.index('B')\nj = s.index('D')\ns = s[:i] + 'X' + s[j:]\nopen('a.txt', 'w').write(s)\nPY",
        &known(&[("/repo/a.txt", Some("ABCDE"))]),
    );
    assert_eq!(found.written, vec![written("/repo/a.txt", "AXDE")]);
}

/// Positions are code points, as Python counts them, not bytes.
#[test]
fn positions_count_code_points() {
    let found = run(
        "python3 - <<'PY'\ns = 'h\u{e9}llo'\nopen('o.txt', 'w').write(s[-3:] + str(s.find('z')) + s[1] + str(s.index('l') - 1))\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o.txt", "llo-1\u{e9}1")]);
}

/// `index` of what is not there raises, and nothing after it runs — unless a
/// handler catches it, in which case the program goes on.
#[test]
fn a_failed_index_raises() {
    let raised = run(
        "python3 - <<'PY'\ns = 'abc'\ns.index('z')\nopen('o.txt', 'w').write('x')\nPY",
        &nothing_known(),
    );
    assert!(raised.written.is_empty(), "{:?}", raised.written);

    let caught = run(
        "python3 - <<'PY'\ns = 'abc'\ntry:\n    i = s.index('z')\nexcept ValueError:\n    i = 0\nopen('o.txt', 'w').write('x')\nPY",
        &nothing_known(),
    );
    assert_eq!(caught.written, vec![written("/repo/o.txt", "x")]);
}

/// A recorded divergence's shape: a helper taking a path and a list of
/// replacements, each asserted to occur once.
#[test]
fn a_helper_looping_over_replacements_it_was_given_is_followed() {
    let found = run(
        "python3 - <<'EOF'\ndef edit(p,R):\n    s=open(p).read()\n    for a,b in R:\n        assert s.count(a)==1,(p,a[:70])\n        s=s.replace(a,b)\n    open(p,'w').write(s)\nedit('k.kt',[\n(\"\"\"one\"\"\",\"\"\"1\"\"\"),\n('two', '2'),\n])\nEOF",
        &known(&[("/repo/k.kt", Some("one two\n"))]),
    );
    assert_eq!(found.written, vec![written("/repo/k.kt", "1 2\n")]);
}

/// A helper from a module this cannot read, handed a path as a plain string —
/// another recorded divergence. The path is refused; the directory handed to
/// `sys.path` forgets only itself, not every file under it.
#[test]
fn a_path_like_string_given_to_an_unknown_function_is_forgotten() {
    let found = run(
        "echo x > /tmp/kept.txt && echo y > a.ts && python3 - <<'PY'\nimport sys; sys.path.insert(0, '/tmp'); from ed import edit\nedit('a.ts', [('y', 'z')])\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/tmp/kept.txt", "x\n")]);
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/a.ts".to_string()),
        why: Why::Python("call ed.edit".to_string()),
    }));
}

/// Shell run from Python is the shell, against the same files: `os.system`'s
/// text is followed, and Python reads what it wrote.
#[test]
fn a_command_python_runs_through_a_shell_is_followed() {
    let found = run(
        "python3 - <<'PY'\nimport os\nos.system('echo hi > a.txt')\ns = open('a.txt').read()\nopen('b.txt', 'w').write(s)\nPY",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/a.txt", "hi\n"),
            written("/repo/b.txt", "hi\n")
        ]
    );
}

/// An argv with no shell between is one command, read by the same tables.
#[test]
fn a_command_python_runs_as_an_argv_is_followed() {
    let found = run(
        "python3 - <<'PY'\nimport subprocess\nopen('a.txt', 'w').write('x')\nsubprocess.run(['sed', '-i', 's/x/y/', 'a.txt'], check=True)\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/a.txt", "y")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);
}

/// `cwd=` is where it runs, and its `cd` stays with it.
#[test]
fn a_commands_working_directory_is_its_own() {
    let found = run(
        "python3 - <<'PY'\nimport subprocess\nsubprocess.run('cd deeper; echo x > c.txt', shell=True, cwd='sub')\nopen('d.txt', 'w').write('y')\nPY",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/sub/deeper/c.txt", "x\n"),
            written("/repo/d.txt", "y")
        ]
    );
}

/// A command the text does not give, or one left running beside the program,
/// is refused.
#[test]
fn an_unknown_or_concurrent_command_is_refused() {
    let unknown = run(
        "python3 - <<'PY'\nimport os\nos.system(os.environ.get('CMD'))\nPY",
        &nothing_known(),
    );
    assert_eq!(
        unknown.unfollowed,
        vec![Unfollowed {
            path: None,
            why: Why::Python("subprocess".to_string()),
        }]
    );

    let concurrent = run(
        "python3 - <<'PY'\nimport subprocess\nsubprocess.Popen(['bash', '-c', 'echo x > e.txt'])\nPY",
        &nothing_known(),
    );
    assert!(concurrent.written.is_empty(), "{:?}", concurrent.written);
    assert!(concurrent.unfollowed.contains(&Unfollowed {
        path: Some("/repo/e.txt".to_string()),
        why: Why::Python("subprocess.Popen".to_string()),
    }));
}

/// An argv word is passed whole: a quote inside it is part of the name.
#[test]
fn an_argv_word_holding_a_quote_stays_one_word() {
    let found = run(
        "python3 - <<'PY'\nimport subprocess\nsubprocess.run(['cp', 'a.txt', \"it's b.txt\"])\nPY",
        &nothing_known(),
    );
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/it's b.txt".to_string()),
            why: Why::NotRead,
        }]
    );
}

/// `re.sub` where Python's `re` and Rust's `regex` mean the same thing: groups,
/// flags, a replacement's own escapes.
#[test]
fn a_regex_substitution_is_followed() {
    let found = run(
        "python3 - <<'PY'\nimport re\ns = open('a.txt').read()\ns = re.sub(r'^(\\w+) = (\\d+)$', r'\\2 = \\1', s, flags=re.M | re.S)\nopen('a.txt', 'w').write(s)\nPY",
        &known(&[("/repo/a.txt", Some("x = 1\ny = 2\n"))]),
    );
    assert_eq!(
        found.written,
        vec![written("/repo/a.txt", "1 = x\n2 = y\n")]
    );

    let named = run(
        "python3 - <<'PY'\nimport re\nopen('o.txt', 'w').write(re.sub(r'(?P<k>a)', r'\\g<k>\\n', 'aXa', count=1) + re.sub('\u{e9}+', 'e', 'caf\u{e9}\u{e9}'))\nPY",
        &nothing_known(),
    );
    assert_eq!(named.written, vec![written("/repo/o.txt", "a\nXacafe")]);
}

/// A compiled pattern, and one built with `re.escape`.
#[test]
fn a_compiled_and_escaped_pattern_is_followed() {
    let found = run(
        "python3 - <<'PY'\nimport re\np = re.compile(re.escape('a.b'))\nopen('o.txt', 'w').write(p.sub('c', 'a.b axb'))\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o.txt", "c axb")]);
}

/// Python's `$` also matches before a final newline, Rust's does not: followed
/// where the text cannot tell them apart, refused where it can.
#[test]
fn a_dollar_before_a_final_newline_is_refused() {
    let refused = run(
        "python3 - <<'PY'\nimport re\nopen('o.txt', 'w').write(re.sub(r'x$', 'y', 'ax\\n'))\nPY",
        &nothing_known(),
    );
    assert_eq!(
        refused.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/o.txt".to_string()),
            why: Why::Python("re $ before a final newline".to_string()),
        }]
    );

    let followed = run(
        "python3 - <<'PY'\nimport re\nopen('o.txt', 'w').write(re.sub(r'x$', 'y', 'ax'))\nPY",
        &nothing_known(),
    );
    assert_eq!(followed.written, vec![written("/repo/o.txt", "ay")]);
}

/// Where the two engines differ, the substitution is refused by what differs.
#[test]
fn a_regex_the_engines_disagree_on_is_refused() {
    for (call, why) in [
        (r"re.sub(r'x*', '-', 'ab')", "re empty match"),
        (r"re.sub(r'(a)\1', 'b', 'aa')", "re backreference"),
        (r"re.sub('a', str.upper, 'a')", "re replacement function"),
        (r"re.sub('a', r'\q', 'a')", "re replacement escape"),
        (r"re.sub('a b', 'c', 'a b', flags=re.X)", "re flag"),
    ] {
        let found = run(
            &format!("python3 - <<'PY'\nimport re\nopen('o.txt', 'w').write({call})\nPY"),
            &nothing_known(),
        );
        assert_eq!(
            found.unfollowed,
            vec![Unfollowed {
                path: Some("/repo/o.txt".to_string()),
                why: Why::Python(why.to_string()),
            }],
            "{call}"
        );
    }
}

/// A replacement's octal escapes, against CPython's own answer: `\101\0` is
/// `'A\x00'`.
#[test]
fn a_replacements_octal_escapes_are_pythons() {
    let found = run(
        "python3 - <<'PY'\nimport re\nopen('o.txt', 'w').write(re.sub('a', r'\\101\\0', 'a'))\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o.txt", "A\u{0}")]);
}

/// A method on text this does not have is a string method: its path-shaped
/// arguments are text being replaced, not files being written.
#[test]
fn a_method_on_unknown_text_writes_none_of_its_arguments() {
    let found = run(
        "python3 - <<'PY'\ns = open('a.txt').read()\ns = s.replace('src/old.ts', 'src/new.ts')\nopen('b.txt', 'w').write(s)\nPY",
        &nothing_known(),
    );
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/b.txt".to_string()),
            why: Why::NotRead,
        }]
    );
}

/// A library call that only reads or computes is handed path-shaped strings all
/// the time; it writes none of them.
#[test]
fn a_pure_library_call_writes_nothing() {
    let found = run(
        "echo x > src/a.ts && python3 - <<'PY'\nimport glob, json, sys\nsys.path.insert(0, '/tmp')\nfiles = glob.glob('src/*.ts')\nprint(json.dumps({'p': 'src/a.ts'}))\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/src/a.ts", "x\n")]);
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);
}

/// A redirect on a group, a subshell or a loop opens the file once, and what
/// each command inside prints without a redirect of its own lands in it.
#[test]
fn a_redirected_group_or_loop_collects_what_its_commands_print() {
    let shown = known(&[("/repo/f", Some("0\n"))]);
    for (command, path, text) in [
        ("{ echo a; echo b; } > f", "/repo/f", "a\nb\n"),
        ("for x in a b; do echo $x; done > f", "/repo/f", "a\nb\n"),
        ("( cd sub; echo x ) >> f", "/repo/f", "0\nx\n"),
        ("{ echo a > g; echo b; } > f", "/repo/f", "b\n"),
        ("{ echo a > g; echo b; } > f", "/repo/g", "a\n"),
        (
            "{ rm -f x; mkdir -p d; echo done; } > f",
            "/repo/f",
            "done\n",
        ),
        (
            "{ set -e; export A=1; echo done; } > f",
            "/repo/f",
            "done\n",
        ),
        ("{ echo a; echo b >&2; } > f 2>/dev/null", "/repo/f", "a\n"),
        ("{ echo a; echo b 1>&2; } > f", "/repo/f", "a\n"),
        ("{ echo a | cat; } > f", "/repo/f", "a\n"),
    ] {
        let found = run(command, &shown);
        let file = found.written.iter().find(|w| w.path == path);
        assert_eq!(
            file,
            Some(&written(path, text)),
            "{command}: {:?}",
            found.unfollowed
        );
    }
}

/// A compound this does not follow still writes where its own redirect says,
/// expanded before its body rebinds anything.
#[test]
fn a_compound_not_followed_is_refused_at_its_own_redirect() {
    let found = run(
        "W=log\n: > \"$W\"\nwhile read x; do W=other; echo $x; done >> \"$W\"",
        &nothing_known(),
    );
    assert!(
        !found.written.iter().any(|w| w.path == "/repo/log"),
        "{:?}",
        found.written
    );
    assert!(
        found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/log"))
    );
}

/// What the evaluator cannot say a command prints leaves the file refused, named
/// for that command.
#[test]
fn a_redirected_group_with_output_it_cannot_follow_is_refused() {
    for (command, why) in [
        ("{ echo a; ./tool; } > f", Why::Program("tool".to_string())),
        (
            "{ echo a | ./tool; echo c; } > f",
            Why::Program("tool".to_string()),
        ),
        ("{ rm -v x; } > f", Why::Program("rm".to_string())),
        ("{ export; } > f", Why::Program("export".to_string())),
        ("{ cd -; } > f", Why::Program("cd".to_string())),
        ("{ echo a & } > f", Why::Background),
        ("{ echo a; } 2> f", Why::Descriptor),
    ] {
        let found = run(command, &nothing_known());
        assert!(
            !found.written.iter().any(|w| w.path == "/repo/f"),
            "{command}: {:?}",
            found.written
        );
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/f") && u.why == why),
            "{command}: {:?}",
            found.unfollowed
        );
    }
}

/// A file is refused once, for what first made it unknown: a later append
/// adds no second reason. Found live: `echo "rc=$?" >> log` after a program
/// wrote the log ranked `expansion` for a file `$?` could not have saved.
#[test]
fn an_append_to_a_refused_file_adds_no_second_reason() {
    let found = run(
        "./build.sh > log 2>&1; echo \"rc=$?\" >> log; echo done >> log",
        &nothing_known(),
    );
    let reasons: Vec<&Why> = found
        .unfollowed
        .iter()
        .filter(|u| u.path.as_deref() == Some("/repo/log"))
        .map(|u| &u.why)
        .collect();
    assert_eq!(reasons, vec![&Why::Program("build.sh".to_string())]);
}

/// A program the tables do not know may write any file, so nothing predicted
/// before it survives it. Found live: a script run from a file (`/tmp/try.sh`,
/// `python3 scripts/insert.py`) rewrote a file the text never named.
#[test]
fn a_program_nobody_knows_forgets_everything_before_it() {
    for program in [
        "./fix.sh",
        "python3 scripts/insert.py a",
        "../scripts/dev cargo fmt",
    ] {
        let found = run(&format!("echo x > a; {program}"), &nothing_known());
        assert!(found.written.is_empty(), "{program}");
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/a") && matches!(u.why, Why::Program(_))),
            "{program}: {:?}",
            found.unfollowed
        );
    }
    let known = run("echo x > a; grep x a; ls", &nothing_known());
    assert_eq!(known.written, vec![written("/repo/a", "x\n")]);
}

fn assuming(before: &[&str], after: &[&str]) -> Assumed {
    Assumed {
        before: before.iter().map(|p| p.to_string()).collect(),
        after: after.iter().map(|p| p.to_string()).collect(),
    }
}

/// What an unknown program withdraws is still predicted, on the condition that
/// it left the file alone. Found live: a Python edit to a Lean file, then
/// `scripts/dev bash -c 'cd lean && lake build'`.
#[test]
fn a_file_an_unknown_program_ran_after_is_predicted_if_it_left_it_alone() {
    let found = run(
        "echo x > a; ./fix.sh; ./fix.sh; ./check.sh",
        &nothing_known(),
    );
    assert!(found.written.is_empty());
    assert_eq!(
        found.conditional,
        vec![Conditional {
            written: written("/repo/a", "x\n"),
            assumed: assuming(&[], &["fix.sh", "check.sh"]),
        }]
    );
}

/// The assumption covers only what the program was not told of: a path in its
/// words or its environment is refused either way. Found live:
/// `rm -f /tmp/rows; OUT=/tmp/rows scripts/dev cargo test` wrote the file it was
/// told, which was predicted absent on the assumption that dev left it alone.
#[test]
fn a_file_an_unknown_program_was_told_of_is_not_assumed_left_alone() {
    for command in [
        "rm -f /tmp/rows; OUT=/tmp/rows ./test.sh",
        "rm -f /tmp/rows; ./test.sh /tmp/rows",
        "rm -f /tmp/rows; ./test.sh --out=/tmp/rows",
        "rm -f /tmp/rows/a; ./test.sh --dir /tmp/rows",
    ] {
        let found = run(command, &nothing_known());
        assert!(
            found.conditional.is_empty(),
            "{command}: {:?}",
            found.conditional
        );
    }
    let untold = run("echo x > /tmp/rows; ./test.sh /tmp/other", &nothing_known());
    assert_eq!(untold.conditional.len(), 1);
}

/// Nor a file the command removed before running it: clearing an output is
/// how a program is asked to write it again. Found live, three times in a day:
/// `rm -f tests/golden/x.jsonl && X_BLESS=1 scripts/dev cargo nextest …`, the
/// golden predicted absent on the assumption that dev left it alone.
#[test]
fn a_file_removed_before_an_unknown_program_is_not_assumed_left_alone() {
    let shown = known(&[
        ("/repo/g", Some("old\n")),
        ("/repo/d/a", Some("x\n")),
        ("/repo/m", Some("m\n")),
    ]);
    for command in [
        "rm -f g && BLESS=1 ./test.sh",
        "rm -rf d; ./build.sh",
        "mv m n; ./test.sh",
        "if [ -r m ]; then rm g; fi; ./test.sh",
    ] {
        let found = run(command, &shown);
        let removed: Vec<&str> = found
            .conditional
            .iter()
            .map(|c| c.written.path.as_str())
            .filter(|path| ["/repo/g", "/repo/d/a", "/repo/m"].contains(path))
            .collect();
        assert!(removed.is_empty(), "{command}: {removed:?}");
    }
    // An edit before a build is still the assumption's to hold.
    let edited = run("echo x > a; ./build.sh", &shown);
    assert_eq!(edited.conditional.len(), 1);
}

/// A checker carried by an unknown program rewrites what the shell tables say
/// it does, here the directory it runs in, without being told one file's name.
/// Found live: a Python edit of `sync.rs`, then `scripts/dev cargo fmt`, and the
/// file predicted unformatted on the assumption that dev left it alone.
#[test]
fn a_file_a_carried_formatter_would_rewrite_is_not_assumed_left_alone() {
    let held = |command: &str| -> Vec<String> {
        run(command, &nothing_known())
            .conditional
            .iter()
            .map(|c| c.written.path.clone())
            .collect()
    };
    let before = "echo x > a.rs; echo y > ../keep; ";
    for formats in [
        "../scripts/dev cargo fmt",
        "scripts/dev bash -c 'cargo fmt --all && cargo clippy'",
        "scripts/dev ruff format",
        "python3 -c 'import subprocess; subprocess.run([\"scripts/dev\", \"black\", \".\"])'",
    ] {
        assert_eq!(
            held(&format!("{before}{formats}")),
            vec!["/keep"],
            "{formats}"
        );
    }
    // Only checked, nothing is rewritten.
    assert_eq!(
        held(&format!("{before}scripts/dev cargo fmt --check")),
        vec!["/repo/a.rs", "/keep"]
    );
    // Where it ran, after its own `cd`, is not known.
    assert!(
        held(&format!(
            "{before}scripts/dev bash -c 'cd lean && cargo fmt'"
        ))
        .is_empty()
    );
}

/// Run before a file is read, the program may have changed what was read.
#[test]
fn a_file_read_after_an_unknown_program_is_predicted_if_it_left_the_input_alone() {
    let files = known(&[("/repo/a", Some("y\n"))]);
    let found = run("./fix.sh; cat a > b", &files);
    assert!(found.written.is_empty());
    assert_eq!(
        found.conditional,
        vec![Conditional {
            written: written("/repo/b", "y\n"),
            assumed: assuming(&["fix.sh"], &[]),
        }]
    );
}

/// The assumption is only that it wrote nothing else: what it writes itself is
/// not known either way, and a file with no unknown program near it is certain.
#[test]
fn what_an_unknown_program_writes_itself_stays_refused() {
    let found = run("./fix.sh > out", &nothing_known());
    assert!(found.conditional.is_empty());
    assert!(
        found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/out")),
        "{:?}",
        found.unfollowed
    );
    assert!(run("echo x > a", &nothing_known()).conditional.is_empty());
}

/// `sys.argv` holds what the shell passed the interpreter, where the text
/// gives it. Found live: `python3 - "$S" <<'EOF'` then `p = sys.argv[1] + …`.
#[test]
fn python_sys_argv_is_what_the_shell_passed() {
    let found = run(
        "S=/tmp/x\npython3 - \"$S\" b <<'PY'\nimport sys\nfrom sys import argv\nopen(sys.argv[1] + '/o', 'w').write(argv[0] + argv[2] + str(len(sys.argv)))\nPY",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![written("/tmp/x/o", "-b3")],
        "{:?}",
        found.unfollowed
    );
    let flagged = run(
        "python3 -u - /tmp/x <<'PY'\nimport sys\nopen(sys.argv[1], 'w').write('x')\nPY",
        &nothing_known(),
    );
    assert!(flagged.written.is_empty(), "{:?}", flagged.written);
}

/// `find` and `index` with a start and an end, as CPython answers them.
#[test]
fn python_find_takes_a_start_and_an_end() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    // Printed by CPython: [s.find('a', 2), s.find('a', -3), s.rfind('a', 0, 4),
    // s.index('b', 1), s.find('a', 9)] for s = 'abcabcab'.
    let found = run(
        &py(
            "s = 'abcabcab'\nopen('o', 'w').write(','.join(str(n) for n in [s.find('a', 2), s.find('a', -3), s.rfind('a', 0, 4), s.index('b', 1), s.find('a', 9)]))",
        ),
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![written("/repo/o", "3,6,3,1,-1")],
        "{:?}",
        found.unfollowed
    );
    let missing = run(
        &py("s = 'abc'\ni = s.index('a', 1)\nopen('o', 'w').write('x')"),
        &nothing_known(),
    );
    assert!(
        missing.written.is_empty(),
        "index raises when it is not there"
    );
}

/// Python's text is what Python reads and writes: its whitespace, and plain
/// UTF-8 opened without `newline=` or another encoding. A text with `\r` in it
/// is refused, since reading turns it into `\n`; so is an `open` taking an
/// argument that changes the bytes.
#[test]
fn python_text_is_what_python_reads_and_writes() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[
        ("/repo/crlf", Some("a\r\nb\r\n")),
        ("/repo/plain", Some("a\nb\n")),
    ]);
    // Expected texts printed by CPython: '\x1ca b\x1f'.strip() and 'a\x1cb'.split().
    let spaces = run(
        &py("open('o', 'w').write('\\x1ca b\\x1f'.strip() + '|' + ','.join('a\\x1cb'.split()))"),
        &shown,
    );
    assert_eq!(spaces.written, vec![written("/repo/o", "a b|a,b")]);
    for program in [
        "open('o', 'w').write(open('crlf').read())",
        "open('o', 'w', newline='\\r\\n').write('x\\n')",
        "open('o', 'w', encoding='latin-1').write('x')",
        "from pathlib import Path\nPath('o').write_text('x', newline='')",
    ] {
        let found = run(&py(program), &shown);
        assert!(found.written.is_empty(), "{program}: {:?}", found.written);
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/o")),
            "{program}"
        );
    }
    let utf8 = run(
        &py("open('o', 'w', encoding='utf-8').write(open('plain', encoding='UTF-8').read())"),
        &shown,
    );
    assert_eq!(utf8.written, vec![written("/repo/o", "a\nb\n")]);
}

/// A dict is followed as Python holds it: insertion order, one place per key,
/// and read back through its items, its keys and `get`.
#[test]
fn a_python_dict_is_followed() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let found = run(
        &py(
            "d = {'b': 1, 'a': 2}\nd['c'] = 3\nd['b'] = 4\nfor k, v in d.items():\n    open(k, 'w').write(str(v) + str(d.get('zz', 0)) + str(len(d)))",
        ),
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/b", "403"),
            written("/repo/a", "203"),
            written("/repo/c", "303"),
        ]
    );
}

/// `json.load` of a shown file, changed, and `json.dump` back: the text is
/// CPython's, byte for byte (each expected text below was printed by it).
#[test]
fn a_json_file_read_changed_and_dumped_is_predicted_exactly() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[("/repo/s.json", Some("{\"b\": 1, \"a\": [1, 2]}"))]);
    let head = "import json\nd = json.load(open('s.json'))\nd['c'] = 'x'\n";
    for (dump, text) in [
        (
            "json.dump(d, open('s.json', 'w'), indent=2)",
            "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ],\n  \"c\": \"x\"\n}",
        ),
        (
            "open('s.json', 'w').write(json.dumps(d))",
            "{\"b\": 1, \"a\": [1, 2], \"c\": \"x\"}",
        ),
    ] {
        let found = run(&py(&format!("{head}{dump}")), &shown);
        assert_eq!(found.written, vec![written("/repo/s.json", text)], "{dump}");
    }
    for (dumps, text) in [
        (
            "json.dumps({'k': 'é\\x7f😀'})",
            "{\"k\": \"\\u00e9\\u007f\\ud83d\\ude00\"}",
        ),
        (
            "json.dumps({'k': 'é'}, ensure_ascii=False)",
            "{\"k\": \"é\"}",
        ),
        (
            "json.dumps({'b': 1, 'a': {}}, sort_keys=True, separators=(',', ':'))",
            "{\"a\":{},\"b\":1}",
        ),
        ("json.dumps([[], {}], indent=0)", "[\n[],\n{}\n]"),
        (
            "json.dumps({1: True, None: None})",
            "{\"1\": true, \"null\": null}",
        ),
    ] {
        let found = run(
            &py(&format!("import json\nopen('o', 'w').write({dumps})")),
            &nothing_known(),
        );
        assert_eq!(found.written, vec![written("/repo/o", text)], "{dumps}");
    }
}

/// What JSON holds that the evaluator does not model is refused, a key that
/// is not there raises, and a dict is shared with its holders and its views
/// as a list is.
#[test]
fn a_python_dict_this_cannot_follow_is_refused() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let shown = known(&[
        ("/repo/s.json", Some("{\"b\": 1}")),
        ("/repo/f.json", Some("{\"x\": 1.5}")),
    ]);
    for program in [
        "import json\nd = json.load(open('f.json'))\nopen('o', 'w').write(json.dumps(d))",
        "import json\nd = json.load(open('s.json'))\ne = d\ne['z'] = 2\nopen('o', 'w').write(json.dumps(d))",
        "import json\nd = json.load(open('s.json'))\nks = d.keys()\nd['z'] = 2\nopen('o', 'w').write(','.join(ks))",
        "d = dict(b=1, a=2)\nopen('o', 'w').write(','.join(d))",
        "import json\nopen('o', 'w').write(json.dumps({'b': 1, 2: 2}, sort_keys=True))",
    ] {
        let found = run(&py(program), &shown);
        assert!(found.written.is_empty(), "{program}: {:?}", found.written);
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/o")),
            "{program}: {:?}",
            found.unfollowed
        );
    }
    let missing = run(
        &py("import json\nd = json.load(open('s.json'))\nd['nope']\nopen('o', 'w').write('x')"),
        &shown,
    );
    assert!(missing.written.is_empty(), "a KeyError ends the program");
}

/// What a block the evaluator does not follow binds is not known after it: a
/// function it defines, and a name its `:=` binds.
#[test]
fn a_python_block_not_followed_leaves_what_it_binds_unknown() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let redefined = run(
        &py(
            "def f():\n    open('a', 'w').write('A')\nif len(open('b').read()):\n    def f():\n        open('a', 'w').write('B')\nf()",
        ),
        &nothing_known(),
    );
    assert!(redefined.written.is_empty(), "{:?}", redefined.written);
    assert!(
        redefined
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/a"))
    );

    let walrus = run(
        &py("line = 'x'\nwhile (line := input()):\n    pass\nopen(line, 'w').write('y')"),
        &nothing_known(),
    );
    assert!(walrus.written.is_empty(), "{:?}", walrus.written);

    let tested = run(
        &py("while open('a', 'w').write('x') and input():\n    pass"),
        &nothing_known(),
    );
    assert!(tested.written.is_empty(), "{:?}", tested.written);
    assert!(
        tested
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/a"))
    );
}

/// A default argument is evaluated once, where the function is defined: a name
/// rebound later does not change it, and a list default is shared by every call.
#[test]
fn a_python_default_argument_is_the_value_at_definition() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let rebound = run(
        &py("name = 'a'\ndef f(p=name):\n    open(p, 'w').write('x')\nname = 'b'\nf()"),
        &nothing_known(),
    );
    assert_eq!(rebound.written, vec![written("/repo/a", "x")]);

    let shared = run(
        &py(
            "def f(acc=[]):\n    acc.append('x')\n    return acc\nf()\nopen('a', 'w').write(''.join(f()))",
        ),
        &nothing_known(),
    );
    assert!(shared.written.is_empty(), "{:?}", shared.written);

    let fixed = run(
        &py(
            "def f(parts=('a',)):\n    for part in parts:\n        open(part, 'w').write('x')\nf()",
        ),
        &nothing_known(),
    );
    assert_eq!(fixed.written, vec![written("/repo/a", "x")]);
}

/// A function defined inside another reads that function's names, which the
/// evaluator does not keep for it: a call to one is not followed, rather than
/// followed against the module's names.
#[test]
fn a_python_function_defined_inside_another_is_not_followed() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let found = run(
        &py(
            "x = 'outer'\ndef f():\n    x = 'inner'\n    def g():\n        open(x, 'w').write('y')\n    g()\nf()",
        ),
        &nothing_known(),
    );
    assert!(
        !found.written.iter().any(|w| w.path == "/repo/outer"),
        "{:?}",
        found.written
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
}

/// A function reached through an expression may write a file handed to it.
#[test]
fn a_python_call_through_an_expression_may_write_its_file_arguments() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let found = run(
        &py("import plugins\nhandlers = plugins.table()\nhandlers['k'](open('a', 'w'))"),
        &nothing_known(),
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(
        found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/a"))
    );
}

/// Paths and open files inside a list or a dict handed to a call this does not
/// follow may each be written.
#[test]
fn a_python_call_this_does_not_follow_may_write_the_files_in_a_list() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    for program in [
        "import fmt\nopen('a.py', 'w').write('x')\nfmt.run(['a.py', 'b.py'])",
        "import fmt\nf = open('a.py', 'w')\nfmt.run({'out': f})",
    ] {
        let found = run(&py(program), &nothing_known());
        assert!(found.written.is_empty(), "{program}: {:?}", found.written);
        assert!(
            found
                .unfollowed
                .iter()
                .any(|u| u.path.as_deref() == Some("/repo/a.py")),
            "{program}: {:?}",
            found.unfollowed
        );
    }
    // Found in history: a list of doc comments, which start with `///`, handed to
    // an edit helper. Text spanning lines names no file.
    let texts = run(
        &py(
            "import edit\nopen('a.py', 'w').write('x')\nedit.apply('b.rs', [('/// one\\n/// two', '/// three\\n/// four')])",
        ),
        &nothing_known(),
    );
    assert_eq!(texts.written, vec![written("/repo/a.py", "x")]);
}

/// A command whose words are not all known may still write any file: what
/// was predicted before it is withdrawn, or conditional on it. Found in
/// history: `subprocess.run(['python3', 'apply.py', f, ...])` in a loop, after a
/// heredoc wrote the file the helper then read and rewrote.
#[test]
fn a_python_subprocess_this_cannot_read_is_an_unknown_program() {
    let py = |body: &str| format!("python3 - <<'PY'\n{body}\nPY");
    let found = run(
        &py(
            "import os, subprocess\nopen('a', 'w').write('x')\nsubprocess.run(['python3', 'apply.py', os.environ.get('F')])",
        ),
        &nothing_known(),
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert_eq!(found.conditional.len(), 1);
    assert_eq!(
        found.conditional[0].assumed.after,
        vec!["python3".to_string()]
    );
}

/// `n<>file` opens it for reading and writing; what goes through it is not modelled.
#[test]
fn a_file_opened_for_reading_and_writing_is_refused() {
    let found = run("cat a 3<> b", &known(&[("/repo/a", Some("x"))]));
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(
        found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/b"))
    );
}

/// `git checkout` rewrites the working tree whether it names a branch or a path;
/// `git status` reads it. Found live: `git checkout -q src/calibration.ts`.
#[test]
fn a_git_command_that_rewrites_the_tree_forgets_it() {
    for git in [
        "git checkout -q a",
        "git checkout main",
        "git stash",
        "git reset --hard",
        "git pull",
    ] {
        let found = run(&format!("echo x > a; {git}"), &nothing_known());
        assert!(found.written.is_empty(), "{git}");
    }
    for git in [
        "git status",
        "git diff a",
        "git log",
        "git add a",
        "git commit -m m",
    ] {
        let found = run(&format!("echo x > a; {git}"), &nothing_known());
        assert_eq!(found.written, vec![written("/repo/a", "x\n")], "{git}");
    }
}

/// A path with a glob in it is every file the pattern matches, which the text
/// cannot list: what lies under its fixed prefix is forgotten. Found live:
/// `ktlint -F "app/src/**/*.kt"` and `rm e2e/zz-shot*.ts`.
#[test]
fn a_glob_forgets_what_lies_under_its_fixed_prefix() {
    for program in ["ktlint -F 'app/src/**/*.kt'", "rm app/src/zz*.kt"] {
        let found = run(
            &format!("echo x > app/src/zz.kt; echo y > other.kt; {program}"),
            &nothing_known(),
        );
        assert_eq!(
            found.written,
            vec![written("/repo/other.kt", "y\n")],
            "{program}"
        );
    }
}

/// `rm -r` takes everything under the directory with it: what the command wrote
/// there is gone, and so is the directory. Found live: `cat > examples/a.rs …;
/// cargo build; rm -r examples`.
#[test]
fn a_recursive_rm_removes_the_files_under_it() {
    let found = run(
        "mkdir -p examples && cat > examples/a.rs <<'EOF'\nfn main() {}\nEOF\nrm -r examples",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![removed("/repo/examples/a.rs"), removed("/repo/examples")]
    );
    assert!(found.unfollowed.is_empty(), "{:?}", found.unfollowed);
}

/// A file written and then removed, as life's screenshot command does with its
/// temporary spec: the end state is that it does not exist, which the text says
/// exactly.
#[test]
fn a_file_written_then_removed_is_predicted_gone() {
    let found = run("echo x > zz.spec.ts; rm zz.spec.ts", &nothing_known());
    assert_eq!(found.written, vec![removed("/repo/zz.spec.ts")]);
    assert!(found.unfollowed.is_empty());
    // Written again after, it holds the new text.
    let again = run("rm -f a; echo y > a", &nothing_known());
    assert_eq!(again.written, vec![written("/repo/a", "y\n")]);
}

/// Under a removed directory there is nothing to read: a later read is of a file
/// that does not exist, not of one the run knows nothing about.
#[test]
fn a_read_under_a_removed_directory_finds_nothing() {
    let found = run("rm -rf d; cat d/x > y", &nothing_known());
    assert!(
        found.unfollowed.contains(&Unfollowed {
            path: Some("/repo/y".to_string()),
            why: Why::Missing,
        }),
        "{:?}",
        found.unfollowed
    );
}

/// A glob is every file it matches, which the text cannot list: still refused.
#[test]
fn a_removed_glob_is_still_refused() {
    let found = run("echo x > e2e/zz.ts; rm e2e/zz*.ts", &nothing_known());
    assert!(found.written.is_empty(), "{:?}", found.written);
}

/// A package script runs code the text does not show, so nothing predicted before
/// it survives; a package manager's query touches nothing. Before this, `pnpm run`
/// and `pnpm test` were taken to touch no files at all.
#[test]
fn a_package_script_forgets_what_came_before_and_a_query_does_not() {
    for script in ["pnpm run fix", "pnpm test", "npm install", "nix run .#gen"] {
        let found = run(&format!("echo x > a; {script}"), &nothing_known());
        assert!(found.written.is_empty(), "{script}");
    }
    for query in [
        "pnpm list",
        "pnpm outdated",
        "nix eval .#x",
        "nix flake show",
    ] {
        let found = run(&format!("echo x > a; {query}"), &nothing_known());
        assert_eq!(found.written, vec![written("/repo/a", "x\n")], "{query}");
    }
}

/// A refusal names the program that ran, not the carrier in front of it:
/// `pnpm exec biome check --write` is biome's write.
#[test]
fn a_refusal_names_the_program_behind_a_carrier() {
    let found = run(
        "echo x > a; pnpm exec biome check --write a",
        &nothing_known(),
    );
    assert!(
        found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/a")
                && u.why == Why::Program("biome".to_string())),
        "{:?}",
        found.unfollowed
    );
}

/// `nix build` writes its out-link, and nothing with `--no-link`.
#[test]
fn nix_build_writes_its_out_link() {
    let found = run("echo x > result; nix build .#x", &nothing_known());
    assert!(found.written.is_empty());
    let kept = run("echo x > result; nix build --no-link .#x", &nothing_known());
    assert_eq!(kept.written, vec![written("/repo/result", "x\n")]);
    let elsewhere = run("echo x > result; nix build -o out .#x", &nothing_known());
    assert_eq!(elsewhere.written, vec![written("/repo/result", "x\n")]);
}

/// What a program that may write anything withdraws is named for the program, in
/// a pipeline or a loop as much as alone: the pipe is not why `a` is unknown.
#[test]
fn a_withdrawal_inside_a_pipeline_names_the_program() {
    let found = run("echo x > a; ./fix.sh | tail -3", &nothing_known());
    assert!(
        found.unfollowed.contains(&Unfollowed {
            path: Some("/repo/a".to_string()),
            why: Why::Program("fix.sh".to_string())
        }),
        "{:?}",
        found.unfollowed
    );
}

/// The text tools, held to what the real ones print: each expected text here
/// was printed by the tools on this machine — the `grep` Claude Code's shell
/// wraps (`ugrep`), macOS's own `/usr/bin/grep` in a `bash -c` child, and GNU
/// coreutils — and all agreed.
#[test]
fn the_text_tools_print_what_the_real_ones_print() {
    let shown = known(&[
        (
            "/repo/a.txt",
            Some("alpha 1\nbeta 2\nAlpha 3\ngamma:x:y\n\nbeta 2\nlast"),
        ),
        ("/repo/b.txt", Some("one\ntwo\n")),
    ]);
    for (case, printed) in [
        ("grep beta a.txt", "beta 2\nbeta 2\n"),
        (
            "grep -v beta a.txt",
            "alpha 1\nAlpha 3\ngamma:x:y\n\nlast\n",
        ),
        ("grep -c beta a.txt", "2\n"),
        (
            "grep -n a a.txt",
            "1:alpha 1\n2:beta 2\n3:Alpha 3\n4:gamma:x:y\n6:beta 2\n7:last\n",
        ),
        ("grep -o 'a[a-z]*' a.txt", "alpha\na\na\namma\na\nast\n"),
        ("grep -x 'beta 2' a.txt", "beta 2\nbeta 2\n"),
        ("grep -F 'a 1' a.txt", "alpha 1\n"),
        ("grep -i alpha a.txt", "alpha 1\nAlpha 3\n"),
        ("grep -l beta a.txt b.txt", "a.txt\n"),
        ("grep o a.txt b.txt", "b.txt:one\nb.txt:two\n"),
        ("grep -h o a.txt b.txt", "one\ntwo\n"),
        ("grep -q beta a.txt && echo yes", "yes\n"),
        ("grep '^[ab]' a.txt", "alpha 1\nbeta 2\nbeta 2\n"),
        ("grep 'a$' a.txt", ""),
        ("cat a.txt | grep beta", "beta 2\nbeta 2\n"),
        ("head -3 a.txt", "alpha 1\nbeta 2\nAlpha 3\n"),
        ("head -n 2 a.txt", "alpha 1\nbeta 2\n"),
        ("tail -2 a.txt", "beta 2\nlast"),
        ("tail -n +5 a.txt", "\nbeta 2\nlast"),
        (
            "cut -d: -f2 a.txt",
            "alpha 1\nbeta 2\nAlpha 3\nx\n\nbeta 2\nlast\n",
        ),
        (
            "cut -d' ' -f1 a.txt",
            "alpha\nbeta\nAlpha\ngamma:x:y\n\nbeta\nlast\n",
        ),
        ("cut -c1-3 a.txt", "alp\nbet\nAlp\ngam\n\nbet\nlas\n"),
        (
            "tr a-z A-Z < a.txt",
            "ALPHA 1\nBETA 2\nALPHA 3\nGAMMA:X:Y\n\nBETA 2\nLAST",
        ),
        (
            "tr -d 'a' < a.txt",
            "lph 1\nbet 2\nAlph 3\ngmm:x:y\n\nbet 2\nlst",
        ),
        (
            "uniq a.txt",
            "alpha 1\nbeta 2\nAlpha 3\ngamma:x:y\n\nbeta 2\nlast\n",
        ),
        (
            "LC_ALL=C sort a.txt",
            "\nAlpha 3\nalpha 1\nbeta 2\nbeta 2\ngamma:x:y\nlast\n",
        ),
        (
            "LC_ALL=C sort -r -u a.txt",
            "last\ngamma:x:y\nbeta 2\nalpha 1\nAlpha 3\n\n",
        ),
        ("basename /x/y/z.txt .txt", "z\n"),
        ("dirname /x/y/z.txt", "/x/y\n"),
        ("head -2 a.txt | tail -1", "beta 2\n"),
        (
            "grep a a.txt | cut -d' ' -f2 | uniq",
            "1\n2\n3\ngamma:x:y\n2\nlast\n",
        ),
    ] {
        let found = run(&format!("{{ {case}; }} > out"), &shown);
        assert_eq!(
            found.written,
            vec![written("/repo/out", printed)],
            "{case}: {:?}",
            found.unfollowed
        );
    }
    // What a loop prints into a pipe is not followed: not the last line of it.
    let looped = run("for i in 1 2; do echo $i; done | tee out", &shown);
    assert!(
        !looped.written.iter().any(|w| w.path == "/repo/out"),
        "{:?}",
        looped.written
    );
    let alternation = run("grep -E 'bet+a|gam' a.txt > out", &shown);
    assert!(
        alternation.written.is_empty(),
        "alternation is refused, not guessed"
    );
}

/// An unquoted heredoc is expanded as bash expands it: a variable the run
/// knows is substituted, a `$` that starts no expansion stays (`.*$"` in a
/// regex), `\$`, `` \` `` and `\\` lose the backslash, and a backslash before
/// a newline joins the lines (in the parser). Each expected text was printed by bash.
/// Found live: a memory edit by `python3 - <<EOF` refused whole for a regex.
#[test]
fn an_unquoted_heredoc_is_expanded_with_what_the_run_knows() {
    let found = run(
        "D=/repo/out; X=v; cat > f <<EOF\na\\\nb \\x \\$X \\\\ ${X} $X. $ $\" end$\nEOF\npython3 - <<EOF\nopen('$D/p', 'w').write(r'^m: .*$' + '${X}')\nEOF",
        &nothing_known(),
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/f", "ab \\x $X \\ v v. $ $\" end$\n"),
            written("/repo/out/p", "^m: .*$v"),
        ]
    );
}

/// What the run does not know is still refused: an unbound name, a form with
/// an operator, a positional or special parameter.
#[test]
fn an_unquoted_heredoc_with_an_unknown_expansion_is_refused() {
    for body in ["$NOPE", "${X:-d}", "$1", "$?", "${#X}"] {
        let found = run(
            &format!("X=v; cat > f <<EOF\n{body}\nEOF"),
            &nothing_known(),
        );
        assert!(found.written.is_empty(), "{body}: {:?}", found.written);
        let found = run(
            &format!("X=v; python3 - <<EOF\nopen('o', 'w').write('{body}')\nEOF"),
            &nothing_known(),
        );
        assert!(found.written.is_empty(), "{body}: {:?}", found.written);
    }
}

/// A redirect's target is expanded before the program starts, so what the
/// program does cannot change which file it writes. Found live: 170 of 254
/// refusals for an expansion were `for d in …; do ./prog > out/$d.json; done`,
/// the loop's own variable forgotten by the unknown program before its
/// redirect was named.
#[test]
fn a_redirect_is_named_before_its_program_runs() {
    let found = run(
        "for d in a b; do ./prog > out/$d.json; done; D=logs; ./prog > $D/x; D=logs; printf -v D q > $D/y",
        &nothing_known(),
    );
    let named: Vec<&str> = found
        .unfollowed
        .iter()
        .filter_map(|u| u.path.as_deref())
        .collect();
    for path in ["/repo/out/a.json", "/repo/out/b.json", "/repo/logs/x"] {
        assert!(named.contains(&path), "{path}: {:?}", found.unfollowed);
    }
    assert!(
        found.unfollowed.iter().all(|u| u.why != Why::Expansion),
        "{:?}",
        found.unfollowed
    );
    // `printf -v D` binds after its redirect was opened under the old `D`
    // (bash: `logs/y` exists afterwards, and `D` is `q`).
    let y = found.written.iter().any(|w| w.path == "/repo/logs/y")
        || found
            .unfollowed
            .iter()
            .any(|u| u.path.as_deref() == Some("/repo/logs/y"));
    assert!(y, "{:?} {:?}", found.written, found.unfollowed);
}

/// A program runs in its own process and cannot change the shell's variables;
/// only a function the text defines runs in this shell and can. So a variable
/// set before an unknown program still names the file a later command writes.
#[test]
fn a_variable_survives_an_unknown_program_but_not_a_function() {
    let named = |command: &str| -> Vec<String> {
        let found = run(command, &nothing_known());
        found
            .written
            .iter()
            .map(|w| w.path.clone())
            .chain(found.unfollowed.iter().filter_map(|u| u.path.clone()))
            .collect()
    };
    assert!(
        named("D=logs; ./prog; echo x > $D/a").contains(&"/repo/logs/a".to_string()),
        "a program"
    );
    let f = named("f() { D=other; }; D=logs; f; echo x > $D/a");
    assert!(
        !f.contains(&"/repo/logs/a".to_string()),
        "a function: {f:?}"
    );
    for rebinds in ["source env.sh", ". env.sh", "eval \"$X\""] {
        let r = named(&format!("D=logs; {rebinds}; echo x > $D/a"));
        assert!(!r.contains(&"/repo/logs/a".to_string()), "{rebinds}: {r:?}");
    }
}

/// `cargo test`, `run`, `nextest` and `bench` run the project's own code, which
/// may write anything: a test that blesses a golden is the common case. Found
/// live four times on 2026-10-01: `rm golden && X_BLESS=1 nix develop -c cargo
/// nextest run …` was predicted, as certain, to leave the golden gone.
#[test]
fn cargo_running_the_projects_code_is_an_unknown_program() {
    let shown = known(&[("/repo/g", Some("old\n"))]);
    for command in [
        "rm g; cargo test",
        "rm g; X_BLESS=1 nix develop -c cargo nextest run --release",
        "rm g; cargo run --bin x",
        "rm g; cargo bench",
    ] {
        let found = run(command, &shown);
        assert!(
            !found.written.iter().any(|w| w.path == "/repo/g"),
            "{command}: {:?}",
            found.written
        );
    }
    // What it was not near is still predicted, on the assumption.
    let found = run("echo x > a; cargo test", &nothing_known());
    assert_eq!(found.conditional.len(), 1, "{:?}", found.conditional);
    // A build only runs build scripts, which write under target/.
    let built = run("rm g; cargo build --release", &shown);
    assert!(built.written.iter().any(|w| w.path == "/repo/g"));
}

/// A program told a file on its stdin is told it as surely as in its words.
/// Found live (2026-10-02): `echo '[{"file":"e2e/ui-pages.spec.ts",…}]' | node
/// satisfy.mjs` rewrote that spec, predicted as left alone.
#[test]
fn a_path_on_a_programs_stdin_is_not_assumed_left_alone() {
    for command in [
        "echo x > e2e/a.ts; echo '[{\"file\":\"e2e/a.ts\",\"line\":3}]' | node fix.mjs",
        "echo x > e2e/a.ts; node fix.mjs <<'EOF'\ne2e/a.ts\nEOF",
    ] {
        let found = run(command, &nothing_known());
        assert!(
            !found
                .conditional
                .iter()
                .any(|c| c.written.path == "/repo/e2e/a.ts"),
            "{command}: {:?}",
            found.conditional
        );
    }
    // Stdin that names nothing leaves the assumption standing.
    let found = run("echo x > a; echo 3 | ./count", &nothing_known());
    assert_eq!(found.conditional.len(), 1, "{:?}", found.conditional);
}

/// `launchctl` starting a job runs the program its plist or its words name,
/// which the text does not show, so it is an unknown program: a file removed
/// before it is not predicted absent after it. Found live (2026-10-04, four
/// findings): `rm -f ~/Library/Logs/life/tcc-probe.log; launchctl bootstrap
/// gui/501 probe.plist` and `launchctl submit -l x -- /tmp/pyprobe.sh`, each
/// predicting a log the job then wrote as absent. A query touches nothing.
#[test]
fn launchctl_starting_a_job_is_an_unknown_program() {
    for script in [
        "rm -f /tmp/probe.log; launchctl bootstrap gui/501 /tmp/x.plist",
        "rm -f /tmp/probe.log; launchctl submit -l x -- /tmp/probe.sh",
        "rm -f /tmp/probe.log; launchctl kickstart -k gui/501/x",
        "rm -f /tmp/probe.log; launchctl load /tmp/x.plist",
    ] {
        let found = run(script, &nothing_known());
        assert!(found.written.is_empty(), "{script}: {:?}", found.written);
        assert!(
            found.conditional.is_empty(),
            "cleared before an unknown program, so not assumed left alone: {script}"
        );
    }
    let found = run("rm -f /tmp/probe.log; launchctl list", &nothing_known());
    assert_eq!(found.written, vec![removed("/tmp/probe.log")]);
}

/// A relative path after a `cd` that only sometimes ran names an unknown file,
/// and a write there withdraws what came before. Found live (2026-10-04):
/// `cd mac-mini && cp f.py /tmp/bak && sed -i … f.py && ! cmp -s … && cd .. &&
/// nix develop …; cp /tmp/bak mac-mini/f.py` predicted the sed's text, and the
/// unconditional restore at the end had put the backup back.
#[test]
fn a_relative_write_after_a_cd_that_only_sometimes_ran_withdraws_what_came_before() {
    let files = known(&[("/repo/d/f.py", Some("a\n")), ("/tmp/bak", None)]);
    let found = run(
        "cd d && cp f.py /tmp/bak && sed -i '' 's|a|b|' f.py && ! cmp -s f.py /tmp/bak && cd .. && echo ok; cp /tmp/bak d/f.py",
        &files,
    );
    assert!(
        found.written.iter().all(|w| w.path != "/repo/d/f.py"),
        "{:?}",
        found.written
    );
}

/// A module's list changed by index inside a call changes in the module: the
/// change is written back where the name is read from, not bound in the call
/// as an assignment would be. Found live (2026-10-05): the `fix(lineno, old,
/// new)` helper below, the commonest edit-helper shape in the corpus, refused
/// every write after it as "item assignment".
#[test]
fn a_modules_list_changed_by_index_inside_a_call_changes_in_the_module() {
    let found = run(
        "python3 - <<'PY'\nL=['a','b']\ndef f():\n    L[0]='z'\nf()\nopen('o','w').write('\\n'.join(L))\nPY",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/o", "z\nb")]);

    let text = "a\nb\nmode := \"sleeping\", place := some \"Home\",\nd\ne\n";
    let files = known(&[("/repo/x.lean", Some(text))]);
    let program = concat!(
        "p='x.lean'; L=open(p).read().split('\\n')\n",
        "def fix(lineno, old, new):\n",
        "    i=lineno-1\n",
        "    for k in range(i, i+4):\n",
        "        if old in L[k]:\n",
        "            L[k]=L[k].replace(old,new,1); return\n",
        "    raise SystemExit(f'not found near {lineno}: {old}')\n",
        "fix(2,'place := some \"Home\",','place := some \"Home\", placeSource := some \"sleep\",')\n",
        "open(p,'w').write('\\n'.join(L)); print('ok')\n",
    );
    let found = run(&format!("python3 - <<'PY'\n{program}PY"), &files);
    assert_eq!(
        found.written,
        vec![written(
            "/repo/x.lean",
            "a\nb\nmode := \"sleeping\", place := some \"Home\", placeSource := some \"sleep\",\nd\ne\n"
        )],
        "{:?}",
        found.unfollowed
    );
}

/// `perl -pi -e` rewrites what sight has shown, as `sed -i` does, where Perl's
/// regex and Rust's agree: the corpus's commonest shapes, `-pi -e 's/…/…/'`
/// and `-0pi -e` across lines, 264 calls on the live census of 2026-10-05.
#[test]
fn perl_in_place_rewrites_what_sight_has_shown() {
    let files = known(&[
        (
            "/repo/deps.dhall",
            Some("pin \"@angular/core\" \"^22.1.7\"\nother\n"),
        ),
        ("/repo/a.rs", Some("x\nuse a;\n  use b;\ny\n")),
    ]);
    let found = run(
        "perl -pi -e 's/(pin \"\\@angular\\/[a-z-]+\" )\"\\^22\\.1\\.[78]\"/$1\"^22.2.0\"/' deps.dhall && perl -0pi -e 's/use a;\\n  use b;/use c;/' a.rs",
        &files,
    );
    assert_eq!(
        found.written,
        vec![
            written(
                "/repo/deps.dhall",
                "pin \"@angular/core\" \"^22.2.0\"\nother\n"
            ),
            written("/repo/a.rs", "x\nuse c;\ny\n"),
        ],
        "{:?}",
        found.unfollowed
    );
    // The backup suffix keeps the old text; what perl does not follow is refused
    // by name under `perl`.
    let found = run(
        "perl -pi.bak -e 's/x/y/' a.rs; perl -pi -e 's/a/$x/' deps.dhall",
        &files,
    );
    assert_eq!(
        found.written,
        vec![
            written("/repo/a.rs.bak", "x\nuse a;\n  use b;\ny\n"),
            written("/repo/a.rs", "y\nuse a;\n  use b;\ny\n"),
        ]
    );
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/deps.dhall".to_string()),
            why: Why::Perl("variable".to_string()),
        }]
    );
}

/// An in-place rewriter's operands are files by its own grammar, a bare `f`
/// included: the tables' path guard (a word needs a `/`, a `~` or an
/// extension) left a write to it neither predicted nor refused. Found live
/// (2026-10-05): `perl -pi -e … f`, the text before predicted as the text
/// after. The same through a Python subprocess, whose argv is handed to the
/// shell quoted.
#[test]
fn a_bare_word_operand_is_a_file_to_an_in_place_rewriter() {
    let files = known(&[("/repo/f", Some("a\n"))]);
    for script in [
        "sed -i '' s/a/b/ f",
        "perl -pi -e 's/a/b/' f",
        "/usr/bin/perl -pi -e 's/a/b/' f",
        "python3 - <<'PY'\nimport subprocess\nsubprocess.run(['perl','-pi','-e','s/a/b/','f'])\nPY",
    ] {
        let found = run(script, &files);
        assert_eq!(found.written, vec![written("/repo/f", "b\n")], "{script}");
    }
    // A Python write, then a subprocess that rewrites the file: the later one wins.
    let found = run(
        "python3 - <<'PY'\nopen('f','w').write('a a\\n')\nimport subprocess\nsubprocess.run(['perl','-pi','-e','s/a/b/g','f'])\nPY",
        &files,
    );
    assert_eq!(found.written, vec![written("/repo/f", "b b\n")]);
    // And one it refuses by name leaves the file refused, not as it was.
    let found = run(
        "printf 'a\\nb\\n' > f; perl -ni -e 'print unless /^a$/' f",
        &files,
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/f".to_string()),
            why: Why::Perl("-n".to_string()),
        }]
    );
}
