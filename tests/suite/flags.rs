//! Every binary parses argv with clap (dev-lint#761): a flag is honoured or
//! refused, never silently replaced by the default. Run against the real
//! binaries, and hermetic: each case is refused while parsing, before anything
//! is read.

use std::process::{Command, Output};

const RANK: &str = env!("CARGO_BIN_EXE_memory-rank");
const TIERS: &str = env!("CARGO_BIN_EXE_memory-tiers");
const LINT: &str = env!("CARGO_BIN_EXE_memory-lint");
const TASK_LINT: &str = env!("CARGO_BIN_EXE_task-lint");

fn run(exe: &str, args: &[&str]) -> Output {
    Command::new(exe)
        .args(args)
        .output()
        .expect("the binary runs")
}

fn refused(out: &Output) -> String {
    assert_eq!(out.status.code(), Some(2), "{out:?}");
    assert!(out.stdout.is_empty(), "a refusal printed a result: {out:?}");
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A bad value must not read as an absent one: `--half-life bogus` once printed
/// the default ranking wearing a parameter's name.
#[test]
fn an_unparseable_value_is_refused_not_defaulted() {
    let text = refused(&run(RANK, &["--half-life", "bogus"]));
    assert!(text.contains("--half-life"), "names the flag: {text}");
    assert!(text.contains("bogus"), "quotes what it got: {text}");
}

#[test]
fn a_flag_with_nothing_after_it_is_refused() {
    refused(&run(RANK, &["--half-life"]));
}

/// The next argument being another flag is a MISSING value, not a bad one, so
/// the error names the flag that lacks it.
#[test]
fn a_following_flag_is_reported_as_a_missing_value() {
    let text = refused(&run(TIERS, &["--breadth", "--lease-days", "30"]));
    assert!(text.contains("--breadth"), "{text}");
}

/// The measured regression: `memory-rank --nonsense` exited 0 with output
/// byte-identical to no arguments, across all nineteen binaries.
#[test]
fn an_unknown_flag_is_refused() {
    let text = refused(&run(RANK, &["--nonsense"]));
    assert!(text.contains("--nonsense"), "quotes what it got: {text}");
}

/// A tool with no flags at all still refuses one, and a stray operand too.
#[test]
fn a_tool_with_no_arguments_takes_none() {
    refused(&run(TASK_LINT, &["--apply"]));
    refused(&run(TASK_LINT, &["extra"]));
}

/// `--` ends the flags: what follows is an operand however it is spelled
/// (memview#1525).
#[test]
fn everything_after_a_double_dash_is_an_operand() {
    let out = run(LINT, &["--", "--not-a-flag"]);
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(!text.contains("unexpected argument"), "{text}");
}

#[test]
fn help_is_answered() {
    let out = run(TIERS, &["--help"]);
    assert!(out.status.success(), "{out:?}");
    assert!(String::from_utf8_lossy(&out.stdout).contains("--lease-days"));
}
