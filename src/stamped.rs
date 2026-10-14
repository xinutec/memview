//! A memory whose content changed while its `modified:` stamp did not.
//!
//! `missing-modified` asks whether a stamp exists; this asks whether it moved, as a
//! script edit leaves it stale. Reads a diff on stdin — not mtimes, and not git,
//! which a hook's `GIT_DIR` would misdirect.

/// The stamp line, as the corpus writes it in frontmatter.
const STAMP: &str = "modified:";

/// Which halves of a memory's provenance its frontmatter does not carry.
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

/// What a memory's frontmatter does not say about its provenance — both
/// fields, `modified:` and `originSessionId`.
pub fn missing(raw: &str) -> Missing {
    let front = raw.split("\n---").next().unwrap_or_default();
    Missing {
        origin: !front.contains("\n  originSessionId:"),
        modified: !front.contains("\n  modified:"),
    }
}

/// A recovered `modified:`: the file's mtime — not the transcript's first write.
pub fn modified_from_mtime(path: &std::path::Path) -> std::io::Result<String> {
    let mtime: chrono::DateTime<chrono::Utc> = std::fs::metadata(path)?.modified()?.into();
    Ok(mtime.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// What a diff says about stamps: memories gone stale, and files that could not be
/// judged — kept apart, or a failure reads as clean.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stale {
    /// Memories whose body moved while their `modified:` did not.
    pub stale: Vec<String>,
    /// Candidates whose body could not be read, so whether they carry a stamp
    /// is unknown. Never silently dropped — see [`unstamped`].
    pub unread: Vec<String>,
}

/// Memories in this diff whose body changed but whose stamp did not, in order.
/// Added and deleted files are not reported, nor files with no `modified:` at all
/// — judged by the stamp's absence, never the name `MEMORY.md`. A file `body`
/// cannot read is reported, not skipped.
pub fn unstamped(diff: &str, body: impl Fn(&str) -> Option<String>) -> Stale {
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
        // `+++`/`---` file headers are not content lines.
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

    // Resolved only for files that would otherwise be reported, so a commit
    // where every stamp moved — the ordinary one — reads no files at all.
    let mut found = Stale::default();
    for name in out {
        match body(&name) {
            Some(text) if missing(&text).modified => {}
            Some(_) => found.stale.push(name),
            None => found.unread.push(name),
        }
    }
    found
}
