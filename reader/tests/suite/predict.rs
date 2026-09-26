//! What a command will leave in the files it writes, predicted from its text and
//! the state it is given.

use std::collections::BTreeMap;

use reader::predict::{Files, Prediction, Unfollowed, Why, Written, needs, predict};

const HOME: &str = "/home/me";
const CWD: &str = "/repo";

fn run(script: &str, files: &Files) -> Prediction {
    let parsed = reader::syntax::parse(script).expect("parses");
    predict(&parsed, CWD, HOME, files)
}

fn written(path: &str, text: &str) -> Written {
    Written {
        path: path.to_string(),
        text: text.to_string(),
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
    let found = run("cat > a <<EOF\nhome is $HOME\nEOF", &nothing_known());
    assert!(found.written.is_empty());
    assert_eq!(found.unfollowed.len(), 1);
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

#[test]
fn a_write_after_or_is_only_sometimes_made() {
    let found = run("false || echo x > a", &nothing_known());
    assert!(found.written.is_empty());
    assert_eq!(found.unfollowed.len(), 1);
}

#[test]
fn a_write_inside_a_loop_is_not_followed() {
    let found = run(
        "for f in a b; do echo x > \"$f\"; done; echo y > c",
        &nothing_known(),
    );
    assert_eq!(found.written, vec![written("/repo/c", "y\n")]);
    assert_eq!(found.unfollowed.len(), 1);
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
                predicted: "x\n".to_string(),
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
            why: Why::Program("sed".to_string()),
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
fn a_program_write_inside_a_loop_is_refused_as_a_compound() {
    let found = run("for f in x; do rm gone.txt; done", &nothing_known());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/gone.txt".to_string()),
            why: Why::Compound,
        }]
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
        "python3 - <<'PY'\nimport sys\nif sys.argv:\n    open('b.txt', 'w').write('x')\nPY",
        &nothing_known(),
    );
    assert!(found.written.is_empty());
    assert_eq!(
        found.unfollowed,
        vec![Unfollowed {
            path: Some("/repo/b.txt".to_string()),
            why: Why::Python("if".to_string()),
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

/// A program inside the nested shell that rewrites a file the outer one wrote
/// makes it unknown — the shape of a heredoc followed by a formatter.
#[test]
fn a_rewrite_inside_a_nested_shell_forgets_the_file() {
    let found = run(
        "echo x > a.txt && bash -c 'sed -i s/x/y/ a.txt'",
        &nothing_known(),
    );
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/a.txt".to_string()),
        why: Why::Program("sed".to_string()),
    }));
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

/// A `cd` inside a region that is not followed still decides where its paths
/// resolve. After it, a pipeline's `cd` is gone with its subshell; one that only
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
        why: Why::Pipeline,
    }));

    let piped = run("cd sub | cat; echo y > b.txt", &nothing_known());
    assert_eq!(piped.written, vec![written("/repo/b.txt", "y\n")]);

    let sometimes = run("false || cd sub; echo y > b.txt", &nothing_known());
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
    assert!(found.written.is_empty(), "{:?}", found.written);
    assert!(found.unfollowed.contains(&Unfollowed {
        path: Some("/repo/a.txt".to_string()),
        why: Why::Program("sed".to_string()),
    }));
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
        "python3 - <<'PY'\nimport os, sys\nos.system(sys.argv[1])\nPY",
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
            why: Why::Program("cp".to_string()),
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
        (
            r"re.sub('a', lambda m: 'b', 'a')",
            "re replacement function",
        ),
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
