//! The `task` service's refusals, pinned by their live wording.
//!
//! These strings are copied verbatim from the service, not
//! paraphrased. The bug they exist to catch was a sentinel that read plausibly
//! and matched nothing the service ever said, so a paraphrase here would
//! reproduce it exactly.

use memview::filing::is_duplicate_refusal;

/// The refusal naming the colliding task by id — what `memory-blame` hit when
/// `health`'s three errors collided with `recall`'s #1554.
const EXACT_SUBJECT: &str = "NOT FILED — #1554 is already open with this exact subject. \
     `task show 1554` to read it, `task edit 1554` if this is an update to it, or re-run \
     with --no-duplicate-check if they really are two tasks.";

/// The other live refusal: a model's read of the open titles, which fires on a
/// similar subject rather than an identical one.
const MODEL_READ: &str = "NOT FILED — a model reading the open titles says this is already \
     one of them. `task show <id>` to check one, or re-run the same command with \
     --no-duplicate-check if this really is different work.";

#[test]
fn both_live_refusal_wordings_are_recognised() {
    assert!(is_duplicate_refusal(EXACT_SUBJECT));
    assert!(is_duplicate_refusal(MODEL_READ));
}

/// The regression, stated as a test rather than a comment: the sentinel this
/// replaced matches NEITHER wording, so the retry it guarded never ran once.
#[test]
fn the_prose_sentinel_it_replaced_matched_neither() {
    for message in [EXACT_SUBJECT, MODEL_READ] {
        assert!(
            !message.contains("already filed"),
            "the old sentinel matched, so this test no longer describes the bug"
        );
        assert!(is_duplicate_refusal(message));
    }
}

/// The guard that matters more than the match: a refusal that must NOT be
/// overruled. Re-running a missing `--priority` with `--no-duplicate-check` files
/// nothing and reports a second error over the first.
#[test]
fn an_unrelated_refusal_is_not_overruled() {
    for message in [
        "a priority is required: pass --priority or --unassessed",
        "the subject is the first argument, not a flag",
        "`--to nobody` needs `--spare \"<why>\"` beside it",
    ] {
        assert!(
            !is_duplicate_refusal(message),
            "must not overrule: {message}"
        );
    }
}
