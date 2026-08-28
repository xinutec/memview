//! Rebuild the record of when each memory was first written.
//!
//!     cargo run --release --bin memory-created
//!
//! ⚠ **This replaces an ad-hoc python script that lived in the DATA directory.**
//! `~/.claude/memview/memory-created.py` wrote its output to `/tmp` and existed
//! in no repository, so nothing could find what made the artefact beside it —
//! "a derived artefact with no repo is one nobody can trace" (memview#1240).
//!
//! ⚠ **And it is more correct than the script**, not merely relocated: the shell
//! arm goes through `reader::shell_files` rather than tokenising a command on
//! `>`, `tee` and `mv`. A heredoc, a `tee -a`, or a redirect glued to the word
//! before it are writes the string test misses.
//!
//! ⚠ **The dates themselves now live in each memory's frontmatter** — see
//! `memory-dated`, which is what a reader should use. This file exists so the
//! frontmatter can be rebuilt, and so a memory since DELETED still has its date
//! recorded somewhere.

use anyhow::{Context, Result};
use serde_json::json;

fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let memory_dir = std::env::var("MEMORY_MARKER")
        .unwrap_or_else(|_| "-Users-pippijn-Code/memory/".to_string());

    let found = memview::blame::all_first_writes(&reader::home::projects_dir(), &memory_dir, &home);
    let out: serde_json::Map<String, serde_json::Value> = found
        .iter()
        .map(|(name, author)| {
            (
                name.clone(),
                json!({ "first": author.at, "session": author.session }),
            )
        })
        .collect();

    let at = reader::home::file("memory-created.json");
    memview::atomic::write(&at, serde_json::to_string_pretty(&out)?.as_bytes())
        .with_context(|| format!("writing {}", at.display()))?;
    println!("{} memories dated → {}", out.len(), at.display());
    Ok(())
}
