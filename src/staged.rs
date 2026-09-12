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
///
/// ⚠ **The NAME cannot be the only way in** (memview#1556). Candidates used to
/// come solely from a `/<basename>/` needle, which finds the other spelling only
/// when both end in the same segment. The corpus repository's two spellings are
/// `/Volumes/Backup/claude` and `~/.claude` — `claude` against `.claude` — so
/// the needle matched one entry, no better spelling existed to compare it to,
/// and **the guard stayed silent across a 390-to-1 discrepancy**: exactly the
/// case it exists for. `staged` now supplies candidates too, by suffix, which
/// consults no name at all.
///
/// ⚠ **A candidate must then be CONFIRMED, or a sibling repo gets reported.**
/// Suffix matching alone is too loose: `memview-web/src/lib.rs` makes
/// `memview-web` look like a spelling of `memview`, and the caller treats an
/// unrelated directory as a misconfiguration and stops checking — turning a
/// working guard into a silent skip. Two routes confirm, and each is exact:
/// the basenames agree, or the two paths CANONICALISE to one directory. The
/// second is what the dotted case needs and the first is what a fixture with
/// paths that do not exist can still use.
pub fn wrong_shape(
    last: &crate::last_writer::LastWriter,
    repo: &str,
    staged: &[String],
) -> Option<Shape> {
    let repo = repo.trim_end_matches('/');
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

    // Route 1 — the repository's own name. Cheap, and right whenever the two
    // spellings end in the same segment.
    if let Some(name) = repo.rsplit('/').next().filter(|n| !n.is_empty()) {
        let needle = format!("/{name}/");
        for key in last.0.keys() {
            if let Some(at) = key.find(&needle) {
                *counts
                    .entry(key[..at + needle.len() - 1].to_string())
                    .or_default() += 1;
            }
        }
    }

    // Route 2 — the paths actually being asked about, which name nothing.
    //
    // ⚠ A HANDFUL of them, not all: this is identifying a prefix, and the
    // twentieth staged path says nothing the first few did not. The record is
    // ~29,000 keys and a commit can stage hundreds.
    for path in staged.iter().take(SAMPLED) {
        let tail = format!("/{}", path.trim_start_matches('/'));
        for key in last.0.keys() {
            if let Some(prefix) = key.strip_suffix(&tail) {
                *counts.entry(prefix.to_string()).or_default() += 1;
            }
        }
    }

    let given = counts.get(repo).copied().unwrap_or(0);
    let (better, better_count) = counts
        .iter()
        .filter(|(prefix, _)| prefix.as_str() != repo && same_repo(prefix, repo))
        .max_by_key(|(_, n)| **n)?;
    (*better_count > given).then(|| Shape {
        given,
        better: better.clone(),
        better_count: *better_count,
    })
}

/// How many staged paths are enough to identify which spelling the record uses.
const SAMPLED: usize = 8;

/// Whether two paths name the same repository.
///
/// ⚠ **Canonicalising is the exact test and the basename is the fallback, not
/// the other way round.** A basename can agree between genuinely different
/// repositories (a clone beside its original), and it can differ between two
/// spellings of one (`claude` and `.claude`). It is kept because it is the only
/// route available when the paths do not exist — which is every fixture, and
/// also a record that outlived the directory it describes.
fn same_repo(a: &str, b: &str) -> bool {
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        return a == b;
    }
    let name = |p: &str| {
        p.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string()
    };
    !name(a).is_empty() && name(a) == name(b)
}
