//! What a session is called to the other sessions on this machine.
//!
//! A conversation has two names and they are not the same thing. The title is
//! what a person renamed it to, lives in the transcript, and is what this console
//! puts on a card ([`crate::past::named`]). The name in here is the one
//! `ListAgents` prints and `SendMessage` resolves, and it lives in a file the CLI
//! keeps per process.
//!
//! They drift because only `-n` at spawn writes this one — a rename reaches the
//! transcript and stops. A console showing the title alone therefore cannot say
//! whether a session is reachable under the name on its card.

use std::path::{Path, PathBuf};

/// Where the CLI keeps one record per running session, named by pid.
///
/// The override exists for the same reason [`crate::past::projects_root`] has
/// one: a test must not read the machine's real sessions, and must not be able
/// to pass by finding them.
pub fn sessions_root() -> PathBuf {
    if let Ok(set) = std::env::var("CLAUDE_SESSIONS_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .join("sessions")
}

/// What the session running as `pid` answers to, when it answers to anything.
///
/// `None` covers every way this can be unknown — no record, unreadable, or a
/// record with no name — because the console has nothing different to say about
/// them: in all three cases it does not know what peers call this session, and
/// guessing would put a name on a card that nothing would reach.
pub fn named(root: &Path, pid: u32) -> Option<String> {
    let raw = std::fs::read_to_string(root.join(format!("{pid}.json"))).ok()?;
    let record: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = record.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}
