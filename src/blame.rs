//! Who wrote a memory, when its own frontmatter does not say.
//!
//! ⚠ **"Unattributable" was true of the FRONTMATTER, not of the corpus.** A
//! memory written by a `cat > … <<'MD'` heredoc skips the stamping path and
//! carries no `originSessionId`, so `lint::passed_for_session` matches it to
//! nobody and it fails no session's gate. That routing is deliberate — but the
//! conclusion drawn from it, that such a memory has no owner and a dashboard is
//! the only answer, does not follow: the transcripts record the write, and
//! `memory-stamp` has recovered the author from them since it was built.
//!
//! This module is that recovery, moved out of the binary so the linter can ask
//! the same question. See `memview#1235`, where a corpus sat uncommittable for
//! about four hours because three unstamped memories were reported to a
//! dashboard nobody was watching, and the session that wrote them was never told.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Who wrote a memory, and when — recovered from one transcript line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Author {
    /// The session id, which is the transcript's filename stem.
    pub session: String,
    /// The write's own timestamp, ISO-8601.
    pub at: String,
}

/// Which session wrote each wanted file, from the transcripts.
///
/// ⚠ **A mention is not a write.** Every session that ever grepped for a memory
/// names it, and the session diagnosing a lint failure names all of them. The
/// filename must be a `file_path` on a writing tool, or a file the *reader* says
/// the command wrote — never merely somewhere in the text.
///
/// Checked against six memories with known authors, all six right: the three of
/// memview#1047, one written by the session that built this, and two whose
/// answer came from frontmatter that session never touched. The sharpest of them
/// is `reference_claude_archive_backup`, which was READ on the day of the check
/// and still attributes to `home` in July.
///
/// `wanted` is a list of file NAMES with their extension — `feedback_x.md`.
pub fn attribute(root: &Path, wanted: &[String], home: &str) -> BTreeMap<String, Author> {
    let mut out: BTreeMap<String, Author> = BTreeMap::new();
    for path in transcripts(root) {
        let session = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            let hit = wanted.iter().find(|n| line.contains(n.as_str()));
            let Some(name) = hit else { continue };
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(at) = row["timestamp"].as_str() else {
                continue;
            };
            let Some(content) = row["message"]["content"].as_array() else {
                continue;
            };
            for item in content {
                if item["type"] != "tool_use" {
                    continue;
                }
                let wrote = match item["name"].as_str() {
                    Some("Write" | "Edit" | "MultiEdit" | "NotebookEdit") => {
                        item["input"]["file_path"]
                            .as_str()
                            .is_some_and(|p| p.ends_with(name.as_str()))
                    }
                    Some("Bash") => item["input"]["command"].as_str().is_some_and(|c| {
                        writes(c, row["cwd"].as_str().filter(|c| !c.is_empty()), home, name)
                    }),
                    _ => false,
                };
                // The EARLIEST write is the creation; later ones are edits, and
                // `originSessionId` means who wrote it first.
                if wrote && out.get(name.as_str()).is_none_or(|a| at < a.at.as_str()) {
                    out.insert(
                        name.clone(),
                        Author {
                            session: session.clone(),
                            at: at.to_string(),
                        },
                    );
                }
            }
        }
    }
    out
}

/// Does `command` WRITE `name`, rather than just mentioning it?
///
/// ⚠ **Asked of the reader rather than of the text.** A scan for `>` followed by
/// the filename gets the common case and then argues with the shell about every
/// other one — `tee`, a heredoc inside `bash -c`, a redirect glued to the word
/// before it. `shell_files` already answers "which files did this command
/// write", and using it here means this cannot drift from the reader the rest of
/// the repo trusts.
pub fn writes(command: &str, cwd: Option<&str>, home: &str, name: &str) -> bool {
    let Ok(parsed) = reader::project::read(command) else {
        return false;
    };
    reader::shell_files::extract(&parsed, cwd, home)
        .files
        .iter()
        .any(|used| used.write && used.path.ends_with(name))
}

/// Every `.jsonl` under the projects root, main-loop and delegated alike.
pub fn transcripts(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "jsonl") {
                out.push(path);
            }
        }
    }
    out
}

/// The marker that makes filing idempotent.
///
/// ⚠ **The nightly runs every day and an unfixed error persists.** Without a
/// marker to find the open task by, one stale `missing-modified` files thirty
/// tasks in a month — and a queue full of duplicates is read as noise and then
/// as nothing, which is the failure this exists to fix, reproduced by the fix.
/// `task add`'s own duplicate check is not enough: it warns and files anyway
/// (`reference_task_add_dedup_warning_still_files`).
pub const MARKER: &str = "[memory-lint]";

/// The subject a filed task carries — the state, since it is the only part
/// anybody reads without opening the task.
pub fn subject(memories: &[String], rule: &str) -> String {
    match memories {
        [one] => format!("{MARKER} `{one}` fails {rule} and the corpus cannot be committed"),
        many => format!(
            "{MARKER} {} of your memories fail the corpus lint and block the nightly commit",
            many.len()
        ),
    }
}

/// This agent's existing open `[memory-lint]` task in `task list --json` output.
///
/// ⚠ **Open only.** A closed task carrying the marker is a fixed error, and
/// refreshing it would reopen a finished conversation; a new error deserves a
/// new task.
pub fn open_task_in(json: &str) -> Option<u64> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let rows = value
        .as_array()
        .cloned()
        .or_else(|| value["tasks"].as_array().cloned())?;
    rows.iter().find_map(|row| {
        let subject = row["subject"].as_str().unwrap_or_default();
        let open = row["status"].as_str() == Some("open");
        (open && subject.contains(MARKER)).then(|| row["id"].as_u64())?
    })
}

/// The earliest write of EVERY memory the transcripts record, not a named few.
///
/// ⚠ **No wanted-list, deliberately.** [`attribute`] answers "who wrote these
/// files", which is what a lint failure needs. Rebuilding the creation record
/// needs the other question — every memory anybody ever wrote — including ones
/// since deleted, whose dates would otherwise be dropped by asking only about
/// files that still exist.
///
/// ⚠ **The shell arm goes through the reader**, where the ad-hoc script this
/// replaces tokenised on `>`, `tee` and `mv` and split on quotes. That guesses;
/// `shell_files` answers. A heredoc, a `tee -a`, or a redirect glued to the
/// word before it are all writes the string test misses.
pub fn all_first_writes(root: &Path, memory_dir: &str, home: &str) -> BTreeMap<String, Author> {
    let mut out: BTreeMap<String, Author> = BTreeMap::new();
    let mut note = |name: &str, at: &str, session: &str| {
        if out.get(name).is_none_or(|a| at < a.at.as_str()) {
            out.insert(
                name.to_string(),
                Author {
                    session: session.to_string(),
                    at: at.to_string(),
                },
            );
        }
    };
    let stem = |path: &str| -> Option<String> {
        let leaf = path.rsplit_once('/')?.1;
        let name = leaf.strip_suffix(".md")?;
        (!name.is_empty() && path.contains(memory_dir)).then(|| name.to_string())
    };

    for path in transcripts(root) {
        let session = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in text.lines() {
            // Cheap reject first: most lines mention no memory at all.
            if !line.contains(memory_dir) {
                continue;
            }
            let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let Some(at) = row["timestamp"].as_str() else {
                continue;
            };
            let Some(content) = row["message"]["content"].as_array() else {
                continue;
            };
            for item in content {
                if item["type"] != "tool_use" {
                    continue;
                }
                match item["name"].as_str() {
                    Some("Write" | "Edit" | "MultiEdit" | "NotebookEdit") => {
                        let path = item["input"]["file_path"]
                            .as_str()
                            .or_else(|| item["input"]["notebook_path"].as_str())
                            .unwrap_or_default();
                        if let Some(name) = stem(path) {
                            note(&name, at, &session);
                        }
                    }
                    Some("Bash") => {
                        let Some(command) = item["input"]["command"].as_str() else {
                            continue;
                        };
                        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
                        let Ok(parsed) = reader::project::read(command) else {
                            continue;
                        };
                        for used in reader::shell_files::extract(&parsed, cwd, home).files {
                            if used.write
                                && let Some(name) = stem(&used.path)
                            {
                                note(&name, at, &session);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}
