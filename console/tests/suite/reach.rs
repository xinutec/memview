//! Stretches a client could not reach the console, as it measured them and
//! handed them over once it could again.

use console::reach::{Reports, Unreached};

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-reach-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir.join("unreached.json")
}

fn stretch(first_failure: i64) -> Unreached {
    Unreached {
        last_answer: Some(first_failure - 3_000),
        first_failure,
        last_failure: first_failure + 50_000,
        answered_again: first_failure + 55_000,
        failures: 11,
    }
}

#[test]
fn a_stretch_handed_over_twice_is_kept_once() {
    // The client resends until it hears back, and the answer can be what is lost.
    let reports = Reports::load(scratch("twice"));
    reports.add(vec![stretch(1_000)]);
    reports.add(vec![stretch(1_000), stretch(90_000)]);
    assert_eq!(reports.all(), vec![stretch(1_000), stretch(90_000)]);
}

#[test]
fn stretches_outlive_the_console() {
    let path = scratch("restart");
    Reports::load(path.clone()).add(vec![stretch(1_000)]);
    assert_eq!(Reports::load(path).all(), vec![stretch(1_000)]);
}

#[test]
fn only_the_newest_are_kept() {
    let reports = Reports::load(scratch("kept"));
    reports.add((0..200).map(|n| stretch(n * 100_000)).collect());
    let all = reports.all();
    assert_eq!(all.len(), console::reach::KEPT);
    assert_eq!(all.last(), Some(&stretch(199 * 100_000)));
}
