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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDelta {
    /// Repo-relative to the code root — `xinutec-infra/plan/backup.dhall`, the
    /// same key the tool-call and shell dimensions use.
    pub path: String,
    /// What the file was called before this commit, when the commit renamed it.
    ///
    /// The only place in the fleet's evidence where a file's two names are known
    /// to be one file. Without it every rename splits a file's history in two
    /// and says nothing about the join.
    pub was: Option<String>,
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
/// **Renames are detected, and that was a correction** (2026-08-02). This ran
/// with `--no-renames` on the reasoning that a rename reported as one is 0
/// added and 0 deleted, so the file would vanish from the record. It does not:
/// `--numstat` still emits a row for it, carrying the touch without the lines.
/// What `--no-renames` actually did was **restate every moved file as a whole
/// deletion and a whole addition** — so a directory reshuffle read as writing
/// the tree from scratch. Measured across four repositories, it inflated lines
/// added by 6–17% and lines *deleted* by 33–41%, and it landed on whoever ran
/// `git mv` rather than on whoever wrote the code.
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
        // `-C` names the directory to work in; it does NOT override an inherited
        // GIT_DIR, which wins and would silently read a different repository.
        // Anything started from a git hook has one set — the miner is normally
        // run from a nightly job, but "normally" is not a guarantee, and a
        // history read from the wrong repo is attributed to the wrong sessions
        // with nothing to give it away.
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_COMMON_DIR")
        .args([
            "log",
            "--numstat",
            "--find-renames",
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
        let (was, path) = renamed(path);
        commit.files.push(FileDelta {
            path: format!("{prefix}/{path}"),
            was: was.map(|old| format!("{prefix}/{old}")),
            added,
            deleted,
        });
    }
    commits
}

/// The old and new names in a `--numstat` path, when it reports a rename.
///
/// Git writes the common parts once and brackets what changed:
/// `code/kubes/ircd/{inspircd => k8s}/ircd.yaml`, and with an empty side when a
/// file moved into or out of a directory — `code/kubes/ircd/{ => k8s}/x.yaml`.
/// When nothing at all is shared it drops the braces: `old.rs => new.rs`.
///
/// Returns `(None, path)` for an ordinary change, so a caller need not ask
/// which shape it got.
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
    // An empty side leaves a doubled separator — `a/{ => b}/c` is `a//c` — which
    // is a different path from the one git meant.
    let join = |middle: &str| format!("{open}{middle}{close}").replace("//", "/");
    (Some(join(from)), join(to))
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
///   line number, a timestamp or a byte count. This has a measured cost: **162
///   of the fleet's 4,697 commits (3.4%) have an all-digit short hash** and can
///   never be attributed. They are counted in `Agents::unattributed` like any
///   other miss, so the gap is reported rather than hidden — and it is the
///   right side of the trade, since the alternative credits one agent with
///   another's work every time a seven-digit number happens to collide.
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
