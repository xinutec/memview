//! What a command was *doing*, one level above what it did to files.
//!
//! The claims here are the ones the file layer cannot make: `cargo test` and
//! `cargo build` name no file and are not the same work, and `echo x > f` names
//! no file in its arguments and is an edit.

use reader::activity::{Activity, of};
use reader::project::read as parse;
use reader::shell_files::extract;

const HOME: &str = "/home/example";
const CWD: &str = "/home/example/Code/health";

/// What one script's commands were doing, in order.
fn doing(script: &str) -> Vec<String> {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    extract(&cmds, Some(CWD), HOME)
        .activities
        .iter()
        .map(|a| a.label().to_string())
        .collect()
}

#[test]
fn one_program_can_be_several_kinds_of_work() {
    // The reason this layer exists: to `shell_ops` these are one command that
    // names no file, and to anybody reading a session they are three things.
    assert_eq!(doing("cargo test -p geo"), ["test"]);
    assert_eq!(doing("cargo build --release"), ["build"]);
    assert_eq!(doing("cargo add serde"), ["install"]);
    assert_eq!(doing("cargo clippy --all-targets"), ["check"]);
}

#[test]
fn a_test_task_is_a_test_run_whatever_tool_runs_it() {
    // `gradle` is a build tool, and `./gradlew :app:testDebugUnitTest` is not a
    // build — asked of the program name alone, every gradle call reads as one.
    assert_eq!(doing("./gradlew :app:testDebugUnitTest"), ["test"]);
    assert_eq!(doing("./gradlew assembleDebug"), ["build"]);
}

#[test]
fn a_redirect_is_an_edit_however_the_command_is_named() {
    // `echo` is navigation; `echo … > f` changed a file, and it is how a large
    // part of the corpus's writing is done.
    assert_eq!(doing("echo hello"), ["navigate"]);
    assert_eq!(doing("echo hello > notes/out.md"), ["edit"]);
    assert_eq!(doing("cat a.ts"), ["inspect"]);
}

#[test]
fn a_deletion_is_a_change_even_when_the_path_is_unknowable() {
    // `rm -rf "$BUILD"` names nothing this can resolve, and something was still
    // removed. The file layer records no path; this records the work.
    assert_eq!(doing("rm -rf \"$BUILD\""), ["edit"]);
    assert_eq!(doing("rm build/out.log"), ["edit"]);
}

#[test]
fn shell_syntax_is_not_work() {
    // The grammar leaves `do`/`done` as ordinary commands on purpose, so that
    // `echo done` cannot end a loop. Counted as activities they would drown the
    // vocabulary in syntax — 29,462 of the corpus's commands are this.
    let seen = doing("for f in a b; do cat $f; done");
    assert!(
        seen.iter().filter(|a| *a == "(not work)").count() >= 1,
        "the keywords are named as not-work, got {seen:?}"
    );
    assert!(seen.contains(&"inspect".to_string()));
}

#[test]
fn the_work_inside_a_wrapper_is_the_work() {
    // A third of the corpus runs through a devshell, and what matters is what
    // it was asked to run — not that a shell was opened.
    assert_eq!(doing("nix develop -c cargo test"), ["test"]);
    // The inner script's work is recorded before the wrapper's own, the same
    // order the operations come back in.
    assert_eq!(
        doing("nix-shell --run 'npx biome check --write src/x.ts'"),
        ["edit", "run"]
    );
}

#[test]
fn what_it_cannot_name_says_so() {
    // A command named by a variable cannot be known, and the honest answer is
    // its own name rather than a guess at a category.
    assert_eq!(doing("$ADB logcat -d"), ["$ADB"]);
    assert!(matches!(
        of(
            &reader::shell_ops::Op::Unknown {
                name: "frobnicate".into()
            },
            &parse("frobnicate x").unwrap().commands[0]
        ),
        Activity::Other { .. }
    ));
}
