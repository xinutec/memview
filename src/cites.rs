//! Claims about tickets, checked against the service that holds them.
//!
//! Two text claims in this repository's orbit have an oracle nobody consults:
//! the `#N` a memory cites, and the subject of a ticket that has been closed.
//! Both are checkable because the task service still holds every id with its
//! status, and neither is checkable offline — so they belong in the nightly
//! beside `memory-blame`, never in the pre-commit gate (memview#1179).
//!
//! ⚠ **Report, never rewrite.** A subject is somebody's sentence about their own
//! work. What a tool can say is "the service disagrees with this"; choosing the
//! new words is not a tool's job (memview#1227).

use std::collections::BTreeSet;

/// A `#N` as written, with whatever qualified it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cited {
    /// The token immediately before the `#`, if any — `memview`, `rxdb`,
    /// `angular/components`.
    pub qualifier: Option<String>,
    pub id: u64,
}

/// Every `#N` a body cites, qualifier included.
///
/// ⚠ **A `#` that opens a heading is not a citation**, which is why a digit must
/// follow it immediately — `## The build` would otherwise read as one. Nor is a
/// CSS colour: `#1a2b3c` carries letters and is refused, and a six-digit run is
/// refused too, since no id in this service is near that long and a false
/// dangling report is worse than a missed one on a check whose whole yield is
/// about five.
pub fn citations(body: &str) -> BTreeSet<Cited> {
    let mut out = BTreeSet::new();
    let bytes = body.as_bytes();
    for (at, _) in body.match_indices('#') {
        let rest = &body[at + 1..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() || digits.len() > 5 {
            continue;
        }
        // A hex colour is digits then letters with no separator; a citation ends
        // at a word boundary.
        let after = rest[digits.len()..].chars().next();
        if after.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            continue;
        }
        // A digit before the `#` is a range or a version, not a reference.
        if at > 0 && bytes[at - 1].is_ascii_digit() {
            continue;
        }
        let head = &body[..at];
        let start = head
            .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '-' || c == '/'))
            .map_or(0, |i| {
                i + head[i..].chars().next().map_or(1, char::len_utf8)
            });
        let qualifier = (start < at).then(|| head[start..].to_string());
        if let Ok(id) = digits.parse() {
            out.insert(Cited { qualifier, id });
        }
    }
    out
}

/// Is this citation about OUR task service, or another project's tracker?
///
/// ⚠ **A qualified id is foreign unless the qualifier is one of ours.** Measured
/// 2026-08-28, both ids the first version reported as dangling were exactly this:
/// `rxdb#7804` and `angular/components#33091` — real issues in other people's
/// repositories, cited correctly, and reported as corpus rot. A check whose whole
/// yield is about five cannot afford two false positives, and "the memory is
/// wrong" is the most expensive thing it can say wrongly.
///
/// `ours` is the set of names the service itself uses, so the rule cannot drift
/// from the service as projects are added.
pub fn is_ours(cited: &Cited, ours: &BTreeSet<String>) -> bool {
    match &cited.qualifier {
        None => true,
        Some(name) => ours.contains(name),
    }
}

/// Phrases that assert a question is still open.
///
/// ⚠ **Narrow on purpose, and this list is the whole rule.** The failure being
/// caught is a subject written as a hypothesis that the investigation then
/// contradicted — and only the *phrased as an open question* form has an oracle.
/// "This number is stale" and "the body contradicts the subject" have none, and
/// a rule that guessed at them would fire on every ticket naming a measurement
/// (memview#1227).
const STILL_OPEN: &[&str] = &[
    "is still unknown",
    "are still unknown",
    "remains unknown",
    "remain unknown",
    "still not known",
    "nobody has said",
    "nobody knows",
    "nobody can say",
    "no one knows",
    "and nothing checks",
    "and nothing uses",
    "and nothing says",
    "and nothing proposes",
    "and nothing reports",
    "and nothing tells",
    "is unclear",
    "we do not know",
    "it is not known",
];

/// Does this subject still ask the question the ticket was closed on?
///
/// Case-insensitive, because a subject may open with the phrase.
pub fn still_asks(subject: &str) -> bool {
    let lower = subject.to_lowercase();
    STILL_OPEN.iter().any(|phrase| lower.contains(phrase))
}

/// A repo-relative path a task's body cites in backticks.
///
/// ⚠ **Backticks and an extension are both required, and that is deliberate.**
/// Prose names directories, module paths and English words containing slashes;
/// requiring a fenced token that ends in a short extension is what keeps this
/// from accusing sentences. Measured 2026-08-30 over 140 open tasks: 98
/// citations, of which 55 were absent — a rate high enough that a looser matcher
/// would drown the real ones.
pub fn cited_paths(body: &str) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '`' {
            i += 1;
            continue;
        }
        let start = i + 1;
        let mut j = start;
        while j < chars.len() && chars[j] != '`' && chars[j] != '\n' {
            j += 1;
        }
        if j >= chars.len() || chars[j] != '`' {
            i = j;
            continue;
        }
        let tok: String = chars[start..j].iter().collect();
        i = j + 1;
        if path_shaped(&tok) {
            out.insert(tok);
        }
    }
    out
}

/// Whether a fenced token reads as a repo-relative file path.
///
/// ⚠ **A URL is not a path** and a leading `/` is not repo-relative — both would
/// be checked against the wrong thing entirely.
fn path_shaped(tok: &str) -> bool {
    if tok.starts_with('/') || tok.contains("://") || tok.contains(' ') {
        return false;
    }
    let Some((dir, file)) = tok.rsplit_once('/') else {
        return false;
    };
    if dir.is_empty() {
        return false;
    }
    let Some((stem, ext)) = file.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !ext.is_empty()
        && ext.len() <= 6
        && ext.chars().all(|c| c.is_ascii_alphanumeric())
        && tok
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-'))
}

/// Where a session's repository is, or `None` when it has no checkout here.
///
/// ⚠ **The FILESYSTEM decides, never `git ls-files`.** health gitignores
/// `tests/golden/**` because those fixtures carry real coordinates; keying on git
/// would report five present-but-untracked files as deleted, which is a wrong
/// accusation about the most sensitive files in the repo (memview#1279).
pub fn repo_of(session: &str, code_root: &std::path::Path) -> Option<std::path::PathBuf> {
    [
        code_root.join(session),
        code_root.join("kubes").join(session),
    ]
    .into_iter()
    .find(|candidate| candidate.is_dir())
}
