//! What was committed, in lines, and which agent to credit it to.
//!
//! The third kind of evidence about who works on something, and the only one
//! that counts *size*. Tool calls and shell commands both say a file was
//! touched; a `Write` of three hundred lines and a one-character `Edit` are
//! each worth 1. Git knows the difference, and knows what survived review.
//!
//! **The hard part is not the lines, it is the attribution.** Every commit in
//! this fleet has the same git author by convention, so the repository cannot
//! say which agent wrote anything. The join runs the other way: a commit hash
//! exists nowhere until the commit is made, so **the session that mentions it
//! first is the session that made it**. Every later mention is somebody quoting
//! a commit that already existed — see [`crate::agents::scan`], where the
//! earliest mention wins.
//!
//! That rule was arrived at by getting it wrong twice, and both wrong versions
//! looked plausible:
//! - matching a **9-character** prefix attributed 1 of 17 commits, because
//!   `git commit` prints a 7-character short hash and the longer string appears
//!   nowhere at all;
//! - matching **any** mention put five agents on one commit, including the
//!   session that was merely reading the history that afternoon.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One commit's effect on one file.
#[derive(Debug, Clone)]
pub struct FileDelta {
    /// Repo-relative to the code root — `xinutec-infra/plan/backup.dhall`, the
    /// same key the tool-call and shell dimensions use.
    pub path: String,
    pub added: usize,
    pub deleted: usize,
}

/// One commit, with the files it changed.
#[derive(Debug, Clone)]
pub struct Commit {
    pub sha: String,
    /// Committer date, ISO-8601. Not used for attribution — a commit can be
    /// authored long before it lands — but kept so a mine can be explained.
    pub when: String,
    pub files: Vec<FileDelta>,
}

/// Every repository directly under the code root.
///
/// Depth two, which is what the layout is: `~/Code/<repo>` and nothing nested
/// (checked — there are no submodules and no worktree `.git` files). A deeper
/// walk would have to skip `node_modules`, and would find nothing for it.
pub fn repositories(code_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(code_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join(".git").exists() {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Every commit in one repository, with per-file line counts.
///
/// `--no-renames` on purpose: a rename reported as one is 0 added and 0
/// deleted, and the file then vanishes from the record of who worked on it.
/// Split into a delete and an add, both agents' work stays visible.
///
/// Merges are skipped (`--no-merges`): a merge commit's diff restates changes
/// already counted against whoever actually made them.
pub fn history(repo: &Path, code_root: &Path) -> Vec<Commit> {
    let prefix = repo
        .strip_prefix(code_root)
        .unwrap_or(repo)
        .to_string_lossy()
        .to_string();
    // \x01 as the field separator: it cannot occur in a commit subject, where a
    // tab or a pipe easily can.
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "log",
            "--numstat",
            "--no-renames",
            "--no-merges",
            "--format=\x01%H\x01%cI",
        ])
        .output();
    let Ok(out) = out else {
        return Vec::new();
    };
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
        // `added \t deleted \t path`, where a binary file reports `-` for both
        // and is skipped: a line count of a PNG is not a measure of anything.
        let mut cols = line.split('\t');
        let (Some(added), Some(deleted), Some(path)) = (cols.next(), cols.next(), cols.next())
        else {
            continue;
        };
        let (Ok(added), Ok(deleted)) = (added.parse(), deleted.parse()) else {
            continue;
        };
        commit.files.push(FileDelta {
            path: format!("{prefix}/{path}"),
            added,
            deleted,
        });
    }
    commits
}

/// Every commit under the code root, newest first within each repository.
pub fn all(code_root: &Path) -> Vec<Commit> {
    repositories(code_root)
        .iter()
        .flat_map(|repo| history(repo, code_root))
        .collect()
}

/// The shortest hash a mention can be recognised by.
///
/// `git commit` prints seven, and so does `git log --oneline`. Requiring more
/// is what made the first attempt find 1 commit of 17.
pub const SHORT: usize = 7;

/// The hash-shaped tokens on a line, as candidate mentions.
///
/// Deliberately fussy, because a false positive credits one agent's work to
/// another:
/// - **7 to 40 characters.** Below seven no hash is printed; above forty is not
///   a git hash at all, which excludes every 64-character sha256 in the corpus
///   — and there are a great many, in lockfiles and nix output.
/// - **At least one letter.** `1234567` is valid hex and is nearly always a
///   line number, a timestamp or a byte count.
/// - **Bounded by non-alphanumerics**, so a hex-looking run inside a longer
///   base64 blob is not mistaken for a hash.
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
        // Step past the character that ended the run, so a `-`-separated uuid
        // yields each of its segments rather than being re-scanned.
        i += 1;
    }
    out
}
