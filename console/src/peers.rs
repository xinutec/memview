//! What a session is called to the other sessions on this machine.
//!
//! Not the title on a card ([`crate::past::named`]) but the name `SendMessage`
//! resolves, kept per process. Only `-n` at spawn sets it, so the two drift.

use std::path::{Path, PathBuf};

/// Where the CLI keeps one record per running session, by pid; overridable for tests.
pub fn sessions_root() -> PathBuf {
    if let Ok(set) = std::env::var("CLAUDE_SESSIONS_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .join("sessions")
}

/// What the session running as `pid` answers to; `None` whenever unknown.
pub fn named(root: &Path, pid: u32) -> Option<String> {
    let raw = std::fs::read_to_string(root.join(format!("{pid}.json"))).ok()?;
    let record: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let name = record.get("name")?.as_str()?.trim();
    (!name.is_empty()).then(|| name.to_string())
}
