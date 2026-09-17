//! One name per operation, in three registers — and the keys a stylesheet
//! selects on.

use reader::reading::naming;
use reader::shell_ops::{GitOp, Op};

/// Every variant, with a representative value. A list rather than something
/// derived, because the point is to notice when a new one appears.
fn every_shape() -> Vec<Op> {
    let path = || "/home/example/a.txt".to_string();
    let paths = || vec![path()];
    vec![
        Op::Read { paths: paths() },
        Op::Write { paths: paths() },
        Op::Remove {
            paths: paths(),
            recursive: false,
        },
        Op::Copy {
            from: paths(),
            to: path(),
        },
        Op::Move {
            from: paths(),
            to: path(),
        },
        Op::Search {
            pattern: "x".into(),
            paths: paths(),
        },
        Op::Run { script: path() },
        Op::Python { source: "1".into() },
        Op::JavaScript { source: "1".into() },
        Op::Sql {
            source: "SELECT 1".into(),
            database: Vec::new(),
        },
        Op::ChangeDir { to: None },
        Op::Git(GitOp::Stage { paths: paths() }),
        Op::Git(GitOp::Alter { paths: paths() }),
        Op::Git(GitOp::Inspect { paths: paths() }),
        Op::Git(GitOp::Other {
            subcommand: "log".into(),
        }),
        Op::Nothing,
        Op::Unknown { name: "k3s".into() },
    ]
}

/// ⚠ **The keys are a CONTRACT WITH A STYLESHEET, which nothing compiles.**
/// `parse-sheet.scss` selects on `[data-kind='shell']`, `[data-kind='unknown']`
/// and five more; renaming a key there is silent — the chip loses its colour and
/// nothing says so. This list is where such a rename becomes loud.
#[test]
fn the_style_keys_are_the_ones_the_stylesheet_selects_on() {
    let known = [
        "read",
        "write",
        "remove",
        "copy",
        "move",
        "search",
        "transform",
        "run",
        "shell",
        "python",
        "javascript",
        "sql",
        "remote",
        "cd",
        "git",
        "nothing",
        "unknown",
        "redirect",
    ];
    for op in every_shape() {
        let key = naming(&op).key;
        assert!(
            known.contains(&key),
            "{key:?} is a new style key — add it to parse-sheet.scss and to this list"
        );
    }
}

/// ⚠ **The word carrying the meaning must not be the one dropped.** `Nothing`
/// means the command touched no FILES. The chip said bare "nothing", which
/// beside `ping` or `task list` reads as "this command did nothing at all".
#[test]
fn nothing_says_what_it_is_nothing_about() {
    let named = naming(&Op::Nothing);
    assert_eq!(named.chip, "no files");
    assert_eq!(named.phrase, "nothing with files");
    assert_ne!(named.chip, "nothing", "the bare word is false on its own");
}

/// A chip shares its line with a host and a depth note on a 412px screen.
#[test]
fn a_chip_is_short_enough_for_a_phone() {
    for op in every_shape() {
        let chip = naming(&op).chip;
        assert!(!chip.is_empty(), "an operation with no chip");
        assert!(
            chip.chars().count() <= 14,
            "chip {chip:?} is {} characters — too wide beside a host",
            chip.chars().count()
        );
    }
}

/// ⚠ **What one shared table buys.** Before it, the console labelled a chip and
/// the viewer labelled a histogram row from two exhaustive matches: both
/// compiled, and both kept compiling while they disagreed.
#[test]
fn every_shape_has_all_three_registers() {
    for op in every_shape() {
        let named = naming(&op);
        assert!(!named.key.is_empty(), "no style key");
        assert!(!named.phrase.is_empty(), "no phrase");
    }
}
