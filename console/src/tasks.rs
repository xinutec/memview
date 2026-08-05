//! The task list a session keeps for itself.
//!
//! Claude Code files these under `~/.claude/tasks/<session-id>/<n>.json`, one
//! small object per task, written as the session creates and updates them. So
//! this is read exactly as [`crate::past`] reads transcripts: off disk, with no
//! control request and nothing asked of the process — a session that is busy for
//! ten minutes answers this instantly, and one that has exited answers it at all.
//!
//! ⚠ **The list and a task's own words are two requests, on purpose.** A
//! description here is not a label: they run to several kilobytes of written-up
//! result, and one live session's 355 tasks are 1.5 MB of them. Sent with the
//! list that is a megabyte and a half onto a phone to draw forty subjects. So
//! [`listed`] carries what a list needs and [`detail`] fetches the prose for the
//! one that was tapped.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where Claude Code keeps them. Overridable for the same reason
/// [`crate::past::projects_root`] is: a test has no `~/.claude` worth writing to.
pub fn tasks_root() -> PathBuf {
    if let Ok(set) = std::env::var("CLAUDE_TASKS_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".claude")
        .join("tasks")
}

/// A task exactly as the file holds it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    id: String,
    subject: String,
    #[serde(default)]
    description: String,
    /// What the session says it is doing while this is underway — its own
    /// present-tense phrasing, which is not always the subject reworded.
    #[serde(default)]
    active_form: Option<String>,
    status: String,
    #[serde(default)]
    blocked_by: Vec<String>,
}

/// One row of the list: what it is, and whether it is done.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Listed {
    pub id: String,
    pub subject: String,
    /// `pending`, `in_progress` or `completed`, in the CLI's own words rather
    /// than a boolean of our own — a third state exists and the client sorts on
    /// it, and an "open" flag would throw away which kind of open it is.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    /// Whether there is prose behind it worth fetching. A task written as a
    /// one-line reminder has none, and offering to open an empty sheet is worse
    /// than not offering.
    pub detailed: bool,
    /// Tasks this one is waiting on, when it is waiting on any.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<String>,
}

/// Every task a session has, oldest first.
///
/// ⚠ **Numerically, not as the directory lists them.** These are named for their
/// ids, so a plain readdir puts `100` before `2` and a list that has run past a
/// hundred reads as shuffled. An id that is not a number sorts last rather than
/// being dropped: the CLI owns this format and may widen it.
pub fn listed(root: &std::path::Path, session: &str) -> Vec<Listed> {
    let mut tasks: Vec<(u64, Listed)> = read(root, session)
        .map(|(stored, ordinal)| {
            (
                ordinal,
                Listed {
                    id: stored.id,
                    subject: stored.subject,
                    status: stored.status,
                    active_form: stored.active_form,
                    detailed: !stored.description.trim().is_empty(),
                    blocked_by: stored.blocked_by,
                },
            )
        })
        .collect();
    tasks.sort_by_key(|(ordinal, _)| *ordinal);
    tasks.into_iter().map(|(_, task)| task).collect()
}

/// What one task says, for the sheet that opened it.
pub fn detail(root: &std::path::Path, session: &str, id: &str) -> Option<String> {
    read(root, session)
        .find(|(stored, _)| stored.id == id)
        .map(|(stored, _)| stored.description)
}

/// Every task file a session has, with the number its name sorts by.
///
/// A file that will not parse is skipped and said so, rather than taken as an
/// empty task: the CLI writes these and a shape we do not know is news, not a
/// blank row.
fn read(root: &std::path::Path, session: &str) -> impl Iterator<Item = (Stored, u64)> {
    let dir = root.join(session);
    let entries = std::fs::read_dir(&dir).into_iter().flatten().flatten();
    entries.filter_map(move |entry| {
        let path = entry.path();
        if path.extension().is_none_or(|it| it != "json") {
            return None;
        }
        let ordinal = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.parse::<u64>().ok())
            .unwrap_or(u64::MAX);
        let raw = std::fs::read_to_string(&path).ok()?;
        match serde_json::from_str::<Stored>(&raw) {
            Ok(stored) => Some((stored, ordinal)),
            Err(failure) => {
                tracing::error!("unreadable task {}: {failure}", path.display());
                None
            }
        }
    })
}
