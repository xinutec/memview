//! A value flag is honoured, or refused — never silently replaced by the default.

use memview::flags::value_of;

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

/// ⚠ The measured regression: this used to return the default, so the run was
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
