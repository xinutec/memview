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
