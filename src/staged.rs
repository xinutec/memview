//! Whose work is in the index, for a commit that is about to take all of it.
//!
//! `git commit` takes the whole index, so another session's staged work rides in
//! your commit. This warns, never blocks: two sessions may edit one file. No git
//! here — a hook's `GIT_DIR` would misdirect it.

/// A staged path whose last recorded writer is not the session committing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foreign {
    pub path: String,
    /// The agent the evidence says wrote it last.
    pub who: String,
    /// Minutes since the epoch, so a caller can say how recent the claim is.
    pub minute: i64,
}

/// Which of `staged` (repo-relative) were last written by somebody other than
/// `me`. The artefact keys absolute paths. A path with no recorded write is not
/// foreign.
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

/// Whether an empty verdict is to be believed, or the repo is recorded under
/// another spelling (`~/Code` resolves to an external volume). Compares the
/// spellings' match counts — mere existence is not the test, since stragglers
/// exist. Candidates come by name and by the staged paths' suffix, and must be
/// confirmed by basename or canonical path, or a sibling repo is reported.
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

    // Route 2: a handful of the staged paths themselves.
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

/// Whether two paths name the same repository: canonically, or by basename when
/// they no longer exist.
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
