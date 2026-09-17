//! Claims about tickets, checked against the service that holds them: the `#N`
//! a memory cites, and the subject of a closed ticket. Neither is checkable
//! offline, so they belong in the nightly (memview#1179). Report, never rewrite
//! (memview#1227).

use std::collections::BTreeSet;

/// A `#N` as written, with whatever qualified it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cited {
    /// The token immediately before the `#`, if any — `memview`, `rxdb`,
    /// `angular/components`.
    pub qualifier: Option<String>,
    pub id: u64,
}

/// Every `#N` a body cites, qualifier included. A `#` opening a heading is not a
/// citation, so a digit must follow immediately; a CSS colour carries letters
/// and is refused; a six-digit run is refused too, since a false dangling report
/// is worse than a missed one on a check whose whole yield is about five.
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

/// Is this citation about OUR task service, or another project's tracker? A
/// qualified id is foreign unless the qualifier is ours: `rxdb#7804` and
/// `angular/components#33091` were reported as corpus rot. `ours` comes from the
/// service.
pub fn is_ours(cited: &Cited, ours: &BTreeSet<String>) -> bool {
    match &cited.qualifier {
        None => true,
        Some(name) => ours.contains(name),
    }
}

/// Phrases that assert a question is still open. Narrow on purpose: only the
/// phrased-as-a-question form has an oracle (memview#1227).
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

/// A repo-relative path a task's body cites in backticks. Backticks and an
/// extension both required, or prose is accused: over 140 open tasks, 98
/// citations of which 55 were absent.
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

/// A URL is not a path and a leading `/` is not repo-relative.
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

/// Where a session's repository is, or `None`. The FILESYSTEM decides, never
/// `git ls-files`: health gitignores its golden fixtures, and keying on git
/// reported five present files as deleted (memview#1279).
pub fn repo_of(session: &str, code_root: &std::path::Path) -> Option<std::path::PathBuf> {
    [
        code_root.join(session),
        code_root.join("kubes").join(session),
    ]
    .into_iter()
    .find(|candidate| candidate.is_dir())
}
