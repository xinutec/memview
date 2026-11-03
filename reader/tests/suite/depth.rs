//! Nesting the reader refuses rather than recurses through. Past it the recursion
//! overflows its stack, which aborts the process — and the console runs the reader
//! on every `Bash` command, so one command took every session with it. Each case
//! runs on a thread with the console's stack, in a debug build whose frames are
//! several times a release build's: the console's margin, and then some.

use reader::predict::{Sight, predict};
use reader::syntax::parse::Reason;
use reader::syntax::python::parse::Reason as PythonReason;

/// The console's runtime threads' stack (`STACK` in `console/src/main.rs`). Before
/// it was raised they had 2 MB, where a release build overflowed at 280 Python
/// brackets and 400 `$(`.
const CONSOLE_STACK: usize = 16 * 1024 * 1024;

/// Run `work` on a thread with [`CONSOLE_STACK`]. An overflow aborts the whole test
/// binary, which is the loud failure wanted.
fn on_console_stack<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(CONSOLE_STACK)
        .spawn(work)
        .expect("a thread")
        .join()
        .expect("the work panicked")
}

/// Every file empty, every directory empty: the evaluator needs something to see.
struct Empty;

impl Sight for Empty {
    fn file(&self, _path: &str) -> Option<Option<String>> {
        Some(Some(String::new()))
    }
    fn dir(&self, _path: &str) -> Option<Option<Vec<String>>> {
        Some(Some(Vec::new()))
    }
}

/// `n` of `open`, the innermost word, `n` of `close`.
fn nested(open: &str, inner: &str, close: &str, n: usize) -> String {
    format!("{}{inner}{}", open.repeat(n), close.repeat(n))
}

#[test]
fn shell_nested_past_the_limit_is_refused_by_name() {
    for script in [
        nested("echo $(", "true", ")", 1000),
        nested("( ", "true", " )", 10_000),
        nested("{ ", "true;", " } ", 10_000),
        nested("if true; then ", "true", "; fi", 10_000),
    ] {
        let refused = on_console_stack(move || reader::syntax::parse(&script).map(drop));
        assert_eq!(refused.map_err(|it| it.reason), Err(Reason::TooDeep));
    }
}

#[test]
fn shell_nested_as_deep_as_scripts_go_still_reads() {
    let script = nested("echo $(", "true", ")", 20);
    let read = on_console_stack(move || reader::syntax::parse(&script).is_ok());
    assert!(read, "twenty substitutions deep is a script, not an attack");
}

#[test]
fn long_flat_commands_are_not_nesting() {
    for script in [
        vec!["true"; 100_000].join(" && "),
        vec!["echo hi"; 100_000].join("\n"),
    ] {
        assert!(on_console_stack(
            move || reader::syntax::parse(&script).is_ok()
        ));
    }
}

/// CPython's own limit: 200 brackets open, and one more is "too many nested
/// parentheses". The reader refuses exactly where the interpreter does.
#[test]
fn python_nested_past_cpythons_limit_never_ran() {
    on_console_stack(|| {
        let most = format!("x = {}", nested("[", "1", "]", 200));
        let past = format!("x = {}", nested("[", "1", "]", 201));
        assert_eq!(reader::python::did_not_run(&most), None);
        assert_eq!(
            reader::python::did_not_run(&past),
            Some("too many nested parentheses")
        );
        assert!(reader::syntax::python::parse::parse(&most).is_ok());
        assert_eq!(
            reader::syntax::python::parse::parse(&past)
                .map(drop)
                .map_err(|it| it.reason),
            Err(PythonReason::TooDeep)
        );
    });
}

/// What overflowed: a nested literal in `python3 -c`, through the evaluator, whose
/// fallback reader recursed once the tree refused it.
#[test]
fn the_evaluator_survives_python_nested_past_the_limit() {
    for depth in [200, 1000, 5000] {
        let command = format!("python3 -c 'x = {}'", nested("[", "1", "]", depth));
        let written = on_console_stack(move || {
            let tree = reader::syntax::parse(&command).expect("the shell reads it");
            predict(&tree, "/tmp", "/tmp", &Empty).written.len()
        });
        assert_eq!(written, 0, "{depth} deep writes nothing");
    }
}
