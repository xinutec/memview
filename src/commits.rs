//! What was committed, in lines, and which agent to credit it to — the only
//! evidence that counts SIZE, and what survived review.
//!
//! Every commit has one git author, so a commit is credited to the session that
//! mentions its hash first — it existed nowhere before.

use anyhow::Context;
use std::path::{Path, PathBuf};
use std::process::Command;

/// One commit's effect on one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDelta {
    /// Repo-relative to the code root — `xinutec-infra/plan/backup.dhall`, the
    /// same key the tool-call and shell dimensions use.
    pub path: String,
    /// What the file was called before this commit, when it renamed it — the only
    /// place in the evidence where a file's two names are known to be one file.
    pub was: Option<String>,
    pub added: usize,
    pub deleted: usize,
}

/// One commit, with the files it changed.
#[derive(Debug, Clone)]
pub struct Commit {
    pub sha: String,
    /// Committer date, ISO-8601. Not used for attribution; kept so a mine can be explained.
    pub when: String,
    pub files: Vec<FileDelta>,
}

/// Every repository directly under the code root. An error is an error, never an
/// empty fleet; only an absent root and a plain file (`ENOTDIR`) are definite.
pub fn repositories(code_root: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(code_root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        other => other.with_context(|| format!("listing {}", code_root.display()))?,
    };
    for entry in entries {
        let path = entry
            .with_context(|| format!("listing {}", code_root.display()))?
            .path();
        // `try_exists`, because `exists()` reports "could not ask" as `false`.
        let is_repo = match path.join(".git").try_exists() {
            Err(e) if e.kind() == std::io::ErrorKind::NotADirectory => false,
            other => other.with_context(|| format!("probing {}", path.display()))?,
        };
        if is_repo {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Every commit in one repository, with per-file line counts — renames detected,
/// merges skipped.
pub fn history(repo: &Path, code_root: &Path) -> anyhow::Result<Vec<Commit>> {
    let prefix = repo
        .strip_prefix(code_root)
        .unwrap_or(repo)
        .to_string_lossy()
        .to_string();
    // \x01 as the field separator: it cannot occur in a commit subject.
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo);
    // Strip every `GIT_*` variable: a hook's `GIT_DIR` beats `-C`.
    for (key, _) in std::env::vars() {
        if key.starts_with("GIT_") {
            cmd.env_remove(key);
        }
    }
    let out = cmd
        .args([
            "log",
            "--numstat",
            "--find-renames",
            "--no-merges",
            "--format=\x01%H\x01%cI",
        ])
        .output()
        // A failure here must not read as an empty history: a spawn failing under
        // load surfaced as "this repository has no commits" (memview#1243).
        .with_context(|| format!("running git log in {}", repo.display()))?;
    anyhow::ensure!(
        out.status.success(),
        "git log in {} exited {}: {}",
        repo.display(),
        out.status,
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);

    let mut commits: Vec<Commit> = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('\x01') {
            let mut parts = rest.split('\x01');
            let (Some(sha), Some(when)) = (parts.next(), parts.next()) else {
                continue;
            };
            commits.push(Commit {
                sha: sha.to_string(),
                when: when.to_string(),
                files: Vec::new(),
            });
            continue;
        }
        let Some(commit) = commits.last_mut() else {
            continue;
        };
        // `added \t deleted \t path`; a binary file reports `-` for both and is skipped.
        let mut cols = line.split('\t');
        let (Some(added), Some(deleted), Some(path)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let (Ok(added), Ok(deleted)) = (added.parse(), deleted.parse()) else {
            continue;
        };
        let (was, path) = renamed(path);
        commit.files.push(FileDelta {
            path: format!("{prefix}/{path}"),
            was: was.map(|old| format!("{prefix}/{old}")),
            added,
            deleted,
        });
    }
    Ok(commits)
}

/// The old and new names in a `--numstat` rename (`a/{b => c}/d`); `(None, path)`
/// otherwise.
pub fn renamed(path: &str) -> (Option<String>, String) {
    let Some((open, rest)) = path.split_once('{') else {
        return match path.split_once(" => ") {
            Some((from, to)) => (Some(from.to_string()), to.to_string()),
            None => (None, path.to_string()),
        };
    };
    let Some((from, rest)) = rest.split_once(" => ") else {
        return (None, path.to_string());
    };
    let Some((to, close)) = rest.split_once('}') else {
        return (None, path.to_string());
    };
    // An empty side leaves a doubled separator — `a/{ => b}/c` is `a//c`.
    let join = |middle: &str| format!("{open}{middle}{close}").replace("//", "/");
    (Some(join(from)), join(to))
}

/// Every commit under the code root, newest first within each repository.
pub fn all(code_root: &Path) -> anyhow::Result<Vec<Commit>> {
    let mut every = Vec::new();
    for repo in repositories(code_root)? {
        every.extend(history(&repo, code_root)?);
    }
    Ok(every)
}

/// The shortest hash a mention can be recognised by: `git commit` prints seven,
/// and requiring more found 1 commit of 17.
pub const SHORT: usize = 7;

/// The hash-shaped tokens on a line: 7 to 40 hex characters with a letter,
/// bounded by non-alphanumerics. Fussy, since a false hit credits the wrong agent.
pub fn hash_candidates(line: &[u8]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        if !line[i].is_ascii_hexdigit() {
            i += 1;
            continue;
        }
        let start = i;
        while i < line.len() && line[i].is_ascii_hexdigit() {
            i += 1;
        }
        let run = &line[start..i];
        let bounded =
            |at: usize| -> bool { line.get(at).is_none_or(|c: &u8| !c.is_ascii_alphanumeric()) };
        if (SHORT..=40).contains(&run.len())
            && run.iter().any(|c| c.is_ascii_alphabetic())
            && (start == 0 || bounded(start - 1))
            && bounded(i)
            && let Ok(text) = std::str::from_utf8(run)
        {
            out.push(text);
        }
        // Step past the character that ended the run, so a `-`-separated uuid yields
        // each of its segments.
        i += 1;
    }
    out
}
