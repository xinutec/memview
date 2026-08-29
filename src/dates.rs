//! When a memory was first written, put back into the memory itself.
//!
//! ⚠ **This was written as a closing window and it is not one.** A creation date
//! exists nowhere but the transcripts — `~/.claude` git history begins
//! 2026-08-14 — and the transcripts were believed to be evaporating. Measured
//! 2026-08-29 against odin's snapshots: **nothing holding a conversation has
//! been deleted since the archive began**, everything ever lost lived in a
//! temp-directory project, and a restore reads back byte-identical
//! (memview#1247, #1240).
//!
//! So the recovery is EXPENSIVE, not perishable, and `memory-created.json` is a
//! cache rather than a rescued artefact. Two bounds are real: sessions predating
//! **2026-07-31** were never archived, and a snapshot lives daily 7 / weekly 4 /
//! monthly 6 / yearly 1. Writing the date into each memory's frontmatter is
//! still right — it makes age an O(1) file read, removes a sidecar, and puts the
//! fact in the one place that is versioned (memview#1210).
//!
//! ⚠ **Insertion, never rewriting.** A memory is somebody's file and most of
//! them already carry `modified:` and `originSessionId:`. Everything here
//! refuses rather than guesses: no `metadata:` block to write under, a `created:`
//! already present, or a date that disagrees with one already there.

use anyhow::{Result, bail};

/// The `created:` a memory's frontmatter already declares.
///
/// ⚠ **Frontmatter only.** A `created:` in the body is prose — `memory-stamp`
/// learned the same thing about `modified:`, and a body match would make this
/// skip a memory that needs stamping.
pub fn created_in(text: &str) -> Option<&str> {
    let front = text.split("\n---").next()?;
    let at = front.find("\n  created:")?;
    let rest = &front[at + "\n  created:".len()..];
    let end = rest.find('\n').unwrap_or(rest.len());
    Some(rest[..end].trim())
}

/// Add `created:` beneath the `type:` line, leaving everything else alone.
///
/// Idempotent: a memory that already carries the same date comes back unchanged.
/// A memory that carries a DIFFERENT one is an error rather than an overwrite —
/// the file's own claim about itself outranks a mined one, and a silent
/// correction is how a recovered date would quietly replace a true one.
pub fn with_created(text: &str, at: &str) -> Result<String> {
    if let Some(existing) = created_in(text) {
        if existing == at {
            return Ok(text.to_string());
        }
        bail!("already says created: {existing}, not {at}");
    }
    // The same anchor `memory-stamp` writes under, so the two agree about where
    // a metadata field belongs.
    let anchor = "\n  type: ";
    let Some(start) = text.find(anchor) else {
        bail!("no `type:` line to write beneath");
    };
    let line_end = text[start + 1..]
        .find('\n')
        .map(|n| start + 1 + n)
        .unwrap_or(text.len());
    let mut out = text.to_string();
    out.insert_str(line_end, &format!("\n  created: {at}"));
    Ok(out)
}
