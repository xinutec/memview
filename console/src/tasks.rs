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
//!
//! [`counts`] is the third and cheapest reader, and the only one that runs
//! unasked: two numbers per session, swept for the whole root at once so the
//! front page can say what is left in a conversation without anybody opening it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::de::DeserializeOwned;
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
    let mut tasks: Vec<(u64, Listed)> = read::<Stored>(root, session)
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

/// How much of a list is left, for a row that is drawn without opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Count {
    /// Anything not `completed` — pending and in progress together, because the
    /// front page's question is "is there work left here", and the difference
    /// between the two kinds of open is what the sheet is for.
    pub open: usize,
    pub total: usize,
}

/// Only the field a count needs. Deliberately not [`Stored`]: the descriptions
/// are most of the bytes on disk — 1.5 MB for the session with 355 tasks — and
/// counting statuses has no use for a single one of them. serde still walks the
/// document, but it allocates nothing for what it skips.
#[derive(Debug, Deserialize)]
struct Progress {
    status: String,
}

/// What a session's directory looked like when it was last counted.
///
/// Three cheap facts off the metadata, and all three because each covers a
/// change the others miss: a task created or deleted moves the count, a status
/// rewritten moves the newest write, and a rewrite landing in the same clock
/// tick as the last one still moves the total size, since no two of the CLI's
/// status words are the same length. The directory's own mtime is not among them
/// — rewriting a file in place does not touch it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mark {
    files: usize,
    newest: Option<std::time::SystemTime>,
    bytes: u64,
}

/// What was counted last time, so an unchanged list is not read twice.
#[derive(Debug, Clone, Copy)]
struct Kept {
    mark: Mark,
    count: Count,
}

/// The counts, kept between sweeps.
///
/// ⚠ **Because counting them all is not free.** Measured on this Mac, in the
/// build the console runs: 516 tasks across eight sessions is **19 ms** warm and
/// **4.8 s** cold, the cold figure being what a poll costs after a restart or
/// once the page cache has let go of 2.2 MB of task files. On a five-second poll
/// that is not a cost to pay for two numbers that change a few times an hour.
///
/// A sweep now stats what it counted before and re-reads only the sessions whose
/// [`Mark`] moved.
#[derive(Debug, Default)]
pub struct Counts {
    held: std::sync::RwLock<BTreeMap<String, Kept>>,
}

impl Counts {
    /// Every session that keeps a list, and how much of each is left.
    ///
    /// ⚠ **One sweep of the root, keyed by session — not a lookup per row.** The
    /// front page draws both the sessions this console runs and the
    /// conversations on disk, and there are far more of the latter than there are
    /// lists: asked one row at a time, most of the calls would be a readdir of a
    /// directory that was never made. The root names exactly the sessions that
    /// have one.
    ///
    /// A session with a directory and nothing in it is absent rather than `0/0`,
    /// which is the rule the client draws by: no list is not an empty list. A
    /// session whose directory has gone is dropped rather than remembered — the
    /// map is rebuilt from what is there each time.
    pub fn sweep(&self, root: &std::path::Path) -> BTreeMap<String, Count> {
        let held = self.held.read().expect("counts poisoned").clone();
        let sessions = std::fs::read_dir(root).into_iter().flatten().flatten();
        let swept: BTreeMap<String, Kept> = sessions
            .filter_map(|entry| {
                let session = entry.file_name().to_str()?.to_string();
                let mark = mark(&entry.path())?;
                let count = match held.get(&session) {
                    Some(kept) if kept.mark == mark => kept.count,
                    _ => count(root, &session),
                };
                Some((session, Kept { mark, count }))
            })
            .collect();
        let counts = swept
            .iter()
            .map(|(session, kept)| (session.clone(), kept.count))
            .collect();
        *self.held.write().expect("counts poisoned") = swept;
        counts
    }
}

/// How a session's directory stands, without opening a single task.
/// `None` when nothing in it is a task — see [`Counts::sweep`].
fn mark(dir: &std::path::Path) -> Option<Mark> {
    let mut mark = Mark {
        files: 0,
        newest: None,
        bytes: 0,
    };
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        if entry.path().extension().is_none_or(|it| it != "json") {
            continue;
        }
        let Ok(about) = entry.metadata() else {
            continue;
        };
        mark.files += 1;
        mark.bytes += about.len();
        mark.newest = mark.newest.max(about.modified().ok());
    }
    (mark.files > 0).then_some(mark)
}

/// One session's list, counted by reading it.
fn count(root: &std::path::Path, session: &str) -> Count {
    let mut count = Count { open: 0, total: 0 };
    for (task, _) in read::<Progress>(root, session) {
        count.total += 1;
        count.open += usize::from(task.status != "completed");
    }
    count
}

/// What one task says, for the sheet that opened it.
pub fn detail(root: &std::path::Path, session: &str, id: &str) -> Option<String> {
    read::<Stored>(root, session)
        .find(|(stored, _)| stored.id == id)
        .map(|(stored, _)| stored.description)
}

/// Every task file a session has, with the number its name sorts by.
///
/// Generic in what is taken off each file, because the three readers want three
/// different amounts of it: the list wants everything but the prose, a count
/// wants one word, and [`detail`] wants the prose alone. What they share is the
/// awkward part — which files count, what their names mean, and what to do with
/// one that will not parse.
///
/// A file that will not parse is skipped and said so, rather than taken as an
/// empty task: the CLI writes these and a shape we do not know is news, not a
/// blank row.
fn read<T: DeserializeOwned>(
    root: &std::path::Path,
    session: &str,
) -> impl Iterator<Item = (T, u64)> {
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
        match serde_json::from_str::<T>(&raw) {
            Ok(stored) => Some((stored, ordinal)),
            Err(failure) => {
                tracing::error!("unreadable task {}: {failure}", path.display());
                None
            }
        }
    })
}
