//! What an index line is for — a tripwire that fires from the line, or a pointer
//! that is found when looked for — and how a line is judged to say which. One
//! definition, shared by `memory-lint` and `memory-tiers`.
//!
//! `memory-roles.json` holds one model's classification; a memory's own `role:`
//! is the author's, and wins — [`role_for`].

/// What an index line is for, from `memory-roles.json`. A tripwire works by
/// being read and never opened, so an estimate averaging the two answers neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Tripwire,
    Pointer,
}

/// What `memory-roles.json` judges one name to be. One reader, because three
/// tools decide demotion from this file; two copies of the string match gave
/// opposite answers for 192 entries (memview#884). `None` is a THIRD state and
/// never a pointer: unjudged is unexamined, and a caller must hold it.
pub fn role_of(roles: &serde_json::Value, name: &str) -> Option<Role> {
    named_role(roles["roles"][name].as_str())
}

/// Whether an index line states its claim, which is what lets it fire on a reader
/// who did not come looking.
///
/// One definition: `memory-lint` and `memory-tiers` must agree about a line, or
/// one offers a demotion the other reports as a defect.
pub fn states_a_claim(label: &str) -> bool {
    label.contains("**") || label.split_whitespace().count() >= 4
}

/// Whether an index line is only a topic word (`gods`, `SMTP`), which names a
/// subject and warns nobody. Stricter than `!states_a_claim`: short tripwires
/// like `no CoA` and `absence≠evid` do their job (memview#1734).
pub fn names_only_a_topic(label: &str) -> bool {
    let mut words = label.split_whitespace();
    matches!((words.next(), words.next()), (Some(word), None)
        if !word.contains("**") && !word.contains(['≠', '=', '→', '<', '>', '!', '?']))
}

/// The vocabulary itself. Anything unrecognised is `None`, never a third kind.
pub fn named_role(text: Option<&str>) -> Option<Role> {
    match text {
        Some("tripwire") => Some(Role::Tripwire),
        Some("pointer") => Some(Role::Pointer),
        _ => None,
    }
}

/// A memory's role, taking the AUTHOR'S declaration over the record. The record
/// is one model's classification memview#884 was pre-registered on, so it cannot
/// be re-run and cannot keep up; every memory written after a pass stayed
/// exempt forever (memview#1537). The frontmatter fixes that without a backfill.
pub fn role_for(declared: Option<&str>, roles: &serde_json::Value, name: &str) -> Option<Role> {
    named_role(declared).or_else(|| role_of(roles, name))
}
