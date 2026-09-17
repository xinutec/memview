//! How this repo's tools read the `task` service's refusals.

/// Is this refusal the DUPLICATE check?
///
/// ⚠ **Keyed on the flag the service names, never on its prose.** The sentinel was
/// the literal `already filed`, which the service does not say — so the retry it
/// guarded never fired once, and a whole agent's corpus errors went unrouted.
/// A flag name is an interface; the sentence around it is not.
///
/// Only overrule on a `true` after establishing the duplicate is spurious —
/// `memory-blame` uses `open_task_in` for that.
pub fn is_duplicate_refusal(message: &str) -> bool {
    message.contains("--no-duplicate-check")
}
