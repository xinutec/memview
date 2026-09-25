//! Python found inside the shell tree, and pointed at where it sits.

use reader::syntax::embed::{Program, Site, python};

/// What was found in `text`: each site's kind, and the program's printed form
/// or why it has none.
fn found(text: &str) -> Vec<(&'static str, String)> {
    let script = reader::syntax::parse(text).expect("shell parses");
    python(&script)
        .into_iter()
        .map(|embedded| {
            let site = match embedded.site {
                Site::Argument(_) => "argument",
                Site::Heredoc(_) => "heredoc",
                Site::HereString(_) => "herestring",
            };
            let program = match embedded.program {
                Program::Text {
                    tree: Ok(module), ..
                } => reader::syntax::python::print(&module),
                Program::Text {
                    tree: Err(refusal), ..
                } => format!("refused {:?}", refusal.reason),
                Program::Expands => "expands".to_string(),
            };
            (site, program)
        })
        .collect()
}

#[test]
fn a_dash_c_argument_is_a_program() {
    assert_eq!(
        found("python3 -u -c 'print(1)'"),
        [("argument", "print(1)\n".to_string())]
    );
}

#[test]
fn a_heredoc_is_the_program_through_a_devshell() {
    assert_eq!(
        found("cd x && nix develop -c python3 - <<'PY'\nprint(1)\nPY"),
        [("heredoc", "print(1)\n".to_string())]
    );
}

#[test]
fn dev_stdin_is_standard_input() {
    assert_eq!(
        found("python3 /dev/stdin <<'PY'\nprint(1)\nPY"),
        [("heredoc", "print(1)\n".to_string())]
    );
}

#[test]
fn a_program_inside_a_substitution_is_found() {
    assert_eq!(
        found("x=$(timeout 5 python3 -c 'print(2)')"),
        [("argument", "print(2)\n".to_string())]
    );
}

#[test]
fn a_herestring_is_the_program() {
    assert_eq!(
        found("python3 <<< 'print(3)'"),
        [("herestring", "print(3)\n".to_string())]
    );
}

/// The shell rewrites an unquoted body before Python sees it: `\\b` there is
/// Python's `\b`, a backspace, and `$f` is whatever `f` held.
#[test]
fn an_unquoted_body_is_read_as_the_shell_hands_it_over() {
    assert_eq!(
        found("python3 <<EOF\nprint('a\\\\b')\nEOF"),
        [("heredoc", "print('a\\x08')\n".to_string())]
    );
    assert_eq!(
        found("python3 <<'EOF'\nprint('a\\\\b')\nEOF"),
        [("heredoc", "print('a\\\\b')\n".to_string())]
    );
    assert_eq!(
        found("python3 <<EOF\nprint('$f')\nEOF"),
        [("heredoc", "expands".to_string())]
    );
    assert_eq!(
        found("python3 -c \"print('$f')\""),
        [("argument", "expands".to_string())]
    );
}

/// The control for each site: the same shapes where Python runs a file, a
/// module, or reads a pipe it was never handed a program on.
#[test]
fn a_script_a_module_and_a_pipe_carry_no_program_here() {
    assert!(found("python3 run.py <<EOF\ndata\nEOF").is_empty());
    assert!(found("python3 -m json.tool <<< '{}'").is_empty());
    assert!(found("echo 'print(1)' | python3").is_empty());
    assert!(found("cat <<'PY'\nprint(1)\nPY").is_empty());
}

#[test]
fn a_program_the_python_tree_cannot_read_is_refused_by_name() {
    let got = found("python3 -c 'class A: pass'");
    assert_eq!(got.len(), 1);
    assert!(got[0].1.starts_with("refused"), "{got:?}");
}
