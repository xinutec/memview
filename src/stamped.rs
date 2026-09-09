//! A memory whose content changed while its `modified:` stamp did not.
//!
//! ⚠ **`memory-lint`'s `missing-modified` cannot see this.** That rule asks
//! whether a stamp EXISTS; this asks whether it MOVED. A memory edited by
//! `sed`, a python script or any tool that is not Edit/Write keeps a stamp
//! describing a version that no longer exists — and the stamp is what recall's
//! age banner reads, so the memory then claims to be fresher than its content
//! (`reference_modified_stamp_is_not_the_file_mtime`).
//!
//! ⚠ **A DIFF, not the filesystem and not git.** mtime was considered and is
//! ruled out by that same memory: it is a property of the filesystem rather than
//! of the corpus and misfires on any synced or restored copy. And git is ruled
//! out inside a hook — a pre-commit hook exports `GIT_DIR` to every child, so a
//! binary running `git -C <other repo>` writes into the COMMITTING repo instead
//! (`reference_a_git_hook_exports_its_repo_to_every_child`). Reading a unified
//! diff on stdin needs neither.

/// The stamp line, as the corpus writes it in frontmatter.
const STAMP: &str = "modified:";

/// Which halves of a memory's provenance its frontmatter does not carry.
///
/// Named rather than a `(bool, bool)`: the two are not interchangeable, and a
/// transposed pair reads as correct at every call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    /// No `originSessionId:` — who wrote it is unrecorded.
    pub origin: bool,
    /// No `modified:` — the age of its claims cannot be judged.
    pub modified: bool,
}

impl Missing {
    /// Whether anything is absent at all.
    pub fn any(self) -> bool {
        self.origin || self.modified
    }
}

/// What a memory's frontmatter does not say about its own provenance.
///
/// ⚠ **Frontmatter only**, for the reason the rest of this module gives: a
/// `modified:` in the body is prose and not a stamp.
///
/// ⚠ **BOTH fields, because asking only about the stamp made `memory-stamp`
/// blind to the field it exists to recover** (memview#1499). Measured
/// 2026-09-09: 719 of 719 memories carried a `modified:` and 707 an
/// `originSessionId`, so every one of the twelve gaps had a stamp, none was
/// returned, and the tool reported "every memory carries a stamp" over them.
///
/// ⚠ **In the library rather than the bin, so a test can reach it.** That
/// defect survived as long as it did because it lived in a `bin` — the same
/// argument `memory-lint`'s SETTLE constant already makes against itself.
pub fn missing(raw: &str) -> Missing {
    let front = raw.split("\n---").next().unwrap_or_default();
    Missing {
        origin: !front.contains("\n  originSessionId:"),
        modified: !front.contains("\n  modified:"),
    }
}

/// Memories in this diff whose body changed but whose stamp did not, in the
/// order they appear.
///
/// A file that is being ADDED is not reported: it has no previous stamp to
/// advance, and a new memory carrying one is the ordinary case. A file being
/// DELETED is not reported either — there is nothing left to be stale.
pub fn unstamped(diff: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut file: Option<String> = None;
    let mut added = false;
    let mut body_changed = false;
    let mut stamp_changed = false;

    // Closing over a file: report it only if its body moved and its stamp did not.
    let mut finish = |file: &mut Option<String>, added: bool, body: bool, stamp: bool| {
        if let Some(name) = file.take()
            && !added
            && body
            && !stamp
        {
            out.push(name);
        }
    };

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            finish(&mut file, added, body_changed, stamp_changed);
            added = false;
            body_changed = false;
            stamp_changed = false;
            // `a/<path> b/<path>` — take the b side, which is the name after
            // any rename.
            file = rest
                .split(" b/")
                .nth(1)
                .filter(|p| p.ends_with(".md"))
                .map(str::to_string);
            continue;
        }
        if file.is_none() {
            continue;
        }
        if line.starts_with("new file mode") {
            added = true;
            continue;
        }
        // ⚠ The `+++`/`---` file headers begin with the same characters as
        // content lines and must not be read as changes; `+++ b/x` would
        // otherwise make every file in the diff look edited.
        if line.starts_with("+++") || line.starts_with("---") || line.starts_with("@@") {
            continue;
        }
        let Some(text) = line.strip_prefix(['+', '-']) else {
            continue;
        };
        if text.trim_start().starts_with(STAMP) {
            stamp_changed = true;
        } else {
            body_changed = true;
        }
    }
    finish(&mut file, added, body_changed, stamp_changed);
    out
}
