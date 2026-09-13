//! How this repo's tools read the `task` service's refusals.
//!
//! One question so far, and it is here rather than in a binary for the reason
//! `blame` gives: a predicate inside a `#[cfg(test)]` block in `src/bin` cannot
//! be exercised through the public API, and `rust-test-module-in-src` refuses it.

/// Is this refusal the DUPLICATE check?
///
/// Only a caller that has already established the duplicate is spurious may act
/// on a `true` — in `memory-blame` that is `open_task_in`, which confirms the
/// agent holds no open task of its own before overruling.
///
/// ⚠ **Keyed on the REMEDY the service names, never on its prose.** The sentinel
/// used to be the literal `already filed`, which the service does not say, so the
/// retry never fired and the fix it belonged to was inert from the day it landed.
///
/// Measured 2026-09-13: `health` had three real corpus errors and they went
/// UNROUTED, because `recall`'s task happened to carry the same subject and the
/// overrule never ran. That is `memview#1235` for the third time, arriving
/// through a different door each time — which is why this is a named, tested
/// function and not a `contains` at the call site.
///
/// A refusal always ends by naming the flag that overrules it. A flag name is an
/// interface; the sentence around it is not.
pub fn is_duplicate_refusal(message: &str) -> bool {
    message.contains("--no-duplicate-check")
}
