//! Whose work is in the index, for a commit that is about to take all of it.
//!
//! ⚠ **`git add <explicit paths>` is not sufficient, and that is the whole
//! point.** `git commit` takes the entire INDEX, so another session staging
//! between your add and your commit puts their work in your commit. Measured
//! 2026-08-29: three files added by name, **nine** staged — six of another
//! session's in-progress work, 486 lines, about to ship under a message about
//! something else. The only thing that stopped it was that session's gate
//! holding the worktree lock.
//!
//! ⚠ **It WARNS and must never block.** Two sessions legitimately edit the same
//! file, and a hard refusal would wedge a shared repo. The failure being silent
//! is the problem; naming it is the fix.
//!
//! ⚠ **No git here.** A pre-commit hook exports `GIT_DIR` to every child, so a
//! checker that shelled out would ask the committing repository about paths it
//! was handed. Paths come in, findings come out.

/// A staged path whose last recorded writer is not the session committing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foreign {
    pub path: String,
    /// The agent the evidence says wrote it last.
    pub who: String,
    /// Minutes since the epoch, so a caller can say how recent the claim is.
    pub minute: i64,
}

/// Which of `staged` were last written by somebody other than `me`.
///
/// `staged` are repo-relative paths as git reports them; `repo` is that
/// repository's ABSOLUTE path.
///
/// ⚠ **The artefact keys paths ABSOLUTELY** — `/Users/pippijn/Code/memview/src/
/// routes/mod.rs`, not `memview/src/routes/mod.rs`. The first version of this
/// joined `repo/path` and matched NOTHING against real data while five fixture
/// tests passed, because the fixture agreed with the same wrong assumption.
/// Check the format against the artefact, never against your own fixture.
///
/// ⚠ **Unknown is NOT foreign.** A path with no recorded write — new, or written
/// by a tool the reader cannot see — yields nothing. A warning that fires on
/// every new file is one people learn to scroll past, which is exactly the
/// failure mode this exists to avoid.
pub fn foreign(
    last: &crate::last_writer::LastWriter,
    repo: &str,
    staged: &[String],
    me: &str,
) -> Vec<Foreign> {
    let mut out = Vec::new();
    for path in staged {
        let full = format!("{}/{}", repo.trim_end_matches('/'), path);
        let Some(wrote) = last.who_wrote(&full) else {
            continue; // unknown is not foreign
        };
        if wrote.who == me {
            continue;
        }
        out.push(Foreign {
            path: path.clone(),
            who: wrote.who.clone(),
            minute: wrote.minute,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// A repository the record barely knows, when it knows another spelling of the
/// same repository far better.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    /// Entries under the path the caller gave.
    pub given: usize,
    /// The prefix for the same repository that the record knows best.
    pub better: String,
    pub better_count: usize,
}

/// Whether an empty verdict should be believed, or is a path-shape mistake.
///
/// ⚠ **The difference between "nothing is foreign" and "nothing matched".** Both
/// make [`foreign`] return empty and only one is good news. `~/Code` is a
/// symlink to an external volume here, so anything that resolves it — `pwd -P`,
/// `realpath`, some `git rev-parse` spellings — yields `/Volumes/…/<repo>/…`
/// while the record holds `~/Code/<repo>/…`.
///
/// ⚠ **Existence is NOT the test, and that was the first attempt.** The record
/// holds BOTH spellings for one repo: measured here, 492 entries under the
/// logical path and 2 under the resolved one. Two stragglers make "does this
/// prefix appear" answer yes for the spelling that matches almost nothing. So
/// this compares the spellings and reports the lopsided ones, which needs no
/// threshold — 2 against 492 is not a judgement call.
pub fn wrong_shape(last: &crate::last_writer::LastWriter, repo: &str) -> Option<Shape> {
    let repo = repo.trim_end_matches('/');
    let name = repo.rsplit('/').next().filter(|n| !n.is_empty())?;
    let mut counts: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    let needle = format!("/{name}/");
    for key in last.0.keys() {
        // The prefix up to and including the repository's own directory.
        if let Some(at) = key.find(&needle) {
            *counts.entry(&key[..at + needle.len() - 1]).or_default() += 1;
        }
    }
    let given = counts.get(repo).copied().unwrap_or(0);
    let (better, better_count) = counts.iter().max_by_key(|(_, n)| **n)?;
    (*better != repo && *better_count > given).then(|| Shape {
        given,
        better: (*better).to_string(),
        better_count: *better_count,
    })
}
