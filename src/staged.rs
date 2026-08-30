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

use std::collections::BTreeMap;

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
    effects: &reader::effects::Effects,
    repo: &str,
    staged: &[String],
    me: &str,
) -> Vec<Foreign> {
    // The last writer of each path, by minute. One pass over the rows rather
    // than a scan per staged file: the artefact holds hundreds of thousands.
    let mut last: BTreeMap<&str, (i64, u32)> = BTreeMap::new();
    for row in &effects.rows {
        if !matches!(row.k, reader::effects::Did::Wrote) {
            continue;
        }
        let Some(path) = row.p.and_then(|p| effects.paths.get(p as usize)) else {
            continue;
        };
        let seen = last.entry(path.as_str()).or_insert((i64::MIN, row.a));
        if row.t >= seen.0 {
            *seen = (row.t, row.a);
        }
    }

    let mut out = Vec::new();
    for path in staged {
        let full = format!("{}/{}", repo.trim_end_matches('/'), path);
        let Some((minute, who)) = last.get(full.as_str()) else {
            continue; // unknown is not foreign
        };
        let Some(name) = effects.agents.get(*who as usize) else {
            continue;
        };
        if name == me {
            continue;
        }
        out.push(Foreign {
            path: path.clone(),
            who: name.clone(),
            minute: *minute,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}
