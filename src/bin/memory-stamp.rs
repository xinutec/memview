//! Give an unstamped memory back its `modified:` and its author.
//!
//!     cargo run --release --bin memory-stamp            # what it would do
//!     cargo run --release --bin memory-stamp -- --apply
//!
//! A memory written with the Write tool is stamped for you. One written with a
//! `cat > … <<'MD'` heredoc is not, and loses two fields: `modified:`, which
//! `memory-lint` reports as an error, and `originSessionId:`, which it does not
//! report at all.
//!
//! ⚠ **The stamp is recoverable and the AUTHOR is the harder half.**
//! `modified:` can always be taken from the file's mtime. Authorship exists only
//! in the transcripts, so a memory written without this has an author for
//! exactly as long as its transcript is reachable — which is why it is worth a
//! tool rather than a habit. It once said "perishable": measured 2026-08-29,
//! transcripts holding a conversation are not being deleted and are archived on
//! odin from 2026-07-31 (memview#1247, #1240). Sessions older than that are gone,
//! and 24 memories name one.
//!
//! ⚠ **This is NOT wired into the gate, deliberately.** `memory-lint` must go on
//! failing on a missing stamp, because the stamp is the only visible symptom of
//! a write that skipped the stamping path — silence it and the authorship loss
//! continues unseen. See `feedback_a_precondition_that_can_pass_wrongly`. What
//! this removes is the twenty minutes of transcript forensics that used to sit
//! between a blocked commit and the fix, in a session that did not write the
//! file (memview #1047: three authors in three hours, each invisible to itself
//! and each blocking somebody else).

use std::path::{Path, PathBuf};

use anyhow::Result;
use memview::atomic;
use memview::blame::{Author, attribute};

fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let apply = std::env::args().any(|a| a == "--apply");
    let dir = std::env::args()
        .nth(1)
        .filter(|a| a != "--apply")
        .unwrap_or_else(|| format!("{home}/.claude/projects/-Users-pippijn-Code/memory"));
    let root = format!("{home}/.claude/projects");

    let wanted = unstamped(Path::new(&dir))?;
    if wanted.is_empty() {
        println!("every memory carries a stamp");
        return Ok(());
    }
    println!("{} unstamped:", wanted.len());

    // `attribute` asks by file NAME, since a transcript records the path the
    // writing session used and not this one's.
    let names: Vec<String> = wanted
        .iter()
        .map(|p| p.file_name().unwrap_or_default().to_string_lossy().into())
        .collect();
    let found = attribute(Path::new(&root), &names, &home);
    for path in &wanted {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        match found.get(name.as_ref()) {
            Some(Author { session, at }) => {
                println!("  {name}\n      {at}  {session}");
                if apply {
                    stamp(path, session, at)?;
                }
            }
            // ⚠ **Named, not guessed.** No transcript claims this file, which
            // means the transcript is gone or something outside a session wrote
            // it. `modified:` could still be taken from the mtime, but writing
            // an author would be inventing one, and a wrong author is worse than
            // an absent one — it sends the next reader to ask the wrong session.
            None => println!("  {name}\n      NO TRANSCRIPT CLAIMS IT — stamp by hand"),
        }
    }
    if !apply {
        println!("\n(dry run — pass --apply to write)");
    }
    Ok(())
}

/// The memories with no `modified:` in their frontmatter.
fn unstamped(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md")
            || path.file_name().is_some_and(|n| n == "MEMORY.md")
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Frontmatter only: a `modified:` in the body is prose, not a stamp.
        let front = text.split("\n---").next().unwrap_or_default();
        if !front.contains("\n  modified:") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

/// Write both fields under the existing `metadata:` block.
fn stamp(path: &Path, session: &str, at: &str) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    let anchor = "\n  type: ";
    let Some(start) = text.find(anchor) else {
        anyhow::bail!("{}: no `type:` line to stamp beneath", path.display());
    };
    let line_end = text[start + 1..]
        .find('\n')
        .map(|n| start + 1 + n)
        .unwrap_or(text.len());
    let mut added = String::new();
    if !text.contains("\n  originSessionId:") {
        added.push_str(&format!("\n  originSessionId: {session}"));
    }
    added.push_str(&format!("\n  modified: {at}"));
    let mut out = text.clone();
    out.insert_str(line_end, &added);
    // ⚠ Rename rather than truncate-then-write: a memory is read by the viewer
    // and by every session at once — see `reference_write_then_rename_or_the_reader_sees_half`.
    atomic::write(path, out.as_bytes())?;
    Ok(())
}
