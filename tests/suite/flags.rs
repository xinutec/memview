//! A value flag is honoured, or refused — never silently replaced by the default.

use memview::flags::{reject_unknown, value_of};

fn argv(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

#[test]
fn an_absent_flag_takes_the_default_and_says_nothing() {
    let args = argv(&["memory-rank"]);
    assert_eq!(value_of(&args, "--half-life", 45_i64).unwrap(), 45);
}

#[test]
fn a_good_value_is_honoured() {
    let args = argv(&["memory-rank", "--half-life", "3"]);
    assert_eq!(value_of(&args, "--half-life", 45_i64).unwrap(), 3);
}

/// ⚠ A bad value must not read as an absent one: defaulting past it makes the run
/// indistinguishable from one that passed no flag at all.
#[test]
fn an_unparseable_value_is_refused_not_defaulted() {
    let args = argv(&["memory-rank", "--half-life", "bogus"]);
    let err = value_of(&args, "--half-life", 45_i64).unwrap_err();
    let text = format!("{err}");
    assert!(text.contains("--half-life"), "names the flag: {text}");
    assert!(text.contains("bogus"), "quotes what it got: {text}");
}

#[test]
fn a_flag_with_nothing_after_it_is_refused() {
    let args = argv(&["memory-rank", "--half-life"]);
    let err = value_of(&args, "--half-life", 45_i64).unwrap_err();
    assert!(format!("{err}").contains("needs a value"));
}

/// ⚠ The next argument being another flag is a MISSING value, not a bad one —
/// otherwise the error names `--lease-days` and sends the reader to the wrong
/// flag entirely.
#[test]
fn a_following_flag_is_reported_as_a_missing_value() {
    let args = argv(&["memory-tiers", "--breadth", "--lease-days", "30"]);
    let err = value_of(&args, "--breadth", 4_usize).unwrap_err();
    let text = format!("{err}");
    assert!(text.contains("needs a value"), "{text}");
    assert!(text.contains("--lease-days"), "names what it found: {text}");
}

/// Two value flags side by side must not read each other's values.
#[test]
fn neighbouring_value_flags_stay_independent() {
    let args = argv(&["memory-tiers", "--breadth", "7", "--lease-days", "30"]);
    assert_eq!(value_of(&args, "--breadth", 4_usize).unwrap(), 7);
    assert_eq!(value_of(&args, "--lease-days", 14_i64).unwrap(), 30);
}

/// ⚠ The measured regression: `memory-rank --nonsense` exited 0 with output
/// byte-identical to no arguments, across all nineteen binaries.
#[test]
fn an_unknown_flag_is_refused_and_the_known_ones_are_named() {
    let args = argv(&["memory-rank", "--nonsense"]);
    let err = reject_unknown(&args, &["--half-life"]).unwrap_err();
    let text = format!("{err}");
    assert!(text.contains("--nonsense"), "quotes what it got: {text}");
    assert!(text.contains("--half-life"), "names what it takes: {text}");
}

#[test]
fn a_known_flag_and_its_operands_pass() {
    let args = argv(&["memory-stamp", "/some/dir", "--apply"]);
    assert!(reject_unknown(&args, &["--apply"]).is_ok());
}

/// `--flag=value` is the same flag as `--flag`.
#[test]
fn an_equals_form_is_recognised() {
    let args = argv(&["memory-rank", "--half-life=30"]);
    assert!(reject_unknown(&args, &["--half-life"]).is_ok());
}

/// ⚠ `--` ends the flags — everything after is an operand however it is spelled.
/// memview#1525: a reader consumed `--` and lost the count behind it.
#[test]
fn everything_after_a_double_dash_is_an_operand() {
    let args = argv(&["memory-lint", "--", "--not-a-flag"]);
    assert!(reject_unknown(&args, &[]).is_ok());
}

/// A tool with no flags at all still refuses one, and says so plainly.
#[test]
fn a_tool_with_no_flags_says_it_takes_none() {
    let args = argv(&["memory-lint", "--json"]);
    let err = reject_unknown(&args, &[]).unwrap_err();
    assert!(format!("{err}").contains("takes no flags"));
}

/// argv[0] is never a flag, however it is spelled.
#[test]
fn the_program_name_is_not_inspected() {
    let args = argv(&["--weird-binary-name"]);
    assert!(reject_unknown(&args, &[]).is_ok());
}

/// A lone `-` is an operand by convention (stdin), not an unknown flag.
#[test]
fn a_lone_dash_is_an_operand() {
    let args = argv(&["staged-check", "-"]);
    assert!(reject_unknown(&args, &[]).is_ok());
}
