//! Put each memory's creation date into the memory, before the evidence goes.
//!
//!     cargo run --release --bin memory-dated            # what it would write
//!     cargo run --release --bin memory-dated -- --apply
//!
//! ⚠ **A one-way window.** A creation date exists nowhere but the transcripts:
//! `~/.claude` git history begins 2026-08-14 and Claude Code prunes its own
//! sessions, so `memview/memory-created.json` is not a cache — it is the
//! surviving output of a recovery that gets less complete every day. Writing it
//! into frontmatter puts the fact in the one place that is versioned, makes age
//! an O(1) file read, and lets `memory-tiers` stop opening a sidecar
//! (memview#1210, #1240).
//!
//! ⚠ **Report first, and `--apply` is the memory session's to run.** memview
//! builds the tools; the corpus is not its to hand-edit. 645 frontmatter writes
//! from a session that does not own the corpus is the concurrent-edit collision
//! at scale — and every memory here is a file some other session may have open.
//!
//! ⚠ **Never overwrites.** A memory that already declares a different `created:`
//! is reported as a conflict and skipped: the file's own claim about itself
//! outranks a mined one.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use memview::dates::{created_in, with_created};

fn main() -> Result<()> {
    let apply = std::env::args().any(|a| a == "--apply");
    let memory_dir = std::env::var("MEMORY_DIR").unwrap_or_else(|_| {
        reader::home::claude_dir()
            .join("projects/-Users-pippijn-Code/memory")
            .to_string_lossy()
            .into_owned()
    });

    let recovered_at = reader::home::file("memory-created.json");
    let recovered: BTreeMap<String, serde_json::Value> = serde_json::from_str(
        &std::fs::read_to_string(&recovered_at)
            .with_context(|| format!("reading {}", recovered_at.display()))?,
    )?;

    let mut wrote = 0usize;
    let mut already = 0usize;
    let mut conflicts: Vec<String> = Vec::new();
    let mut undated: Vec<String> = Vec::new();

    let mut names: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&memory_dir)?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md")
            || path.file_name().is_some_and(|n| n == "MEMORY.md")
        {
            continue;
        }
        names.push(path.to_string_lossy().into_owned());
    }
    names.sort();

    for path in &names {
        let path = std::path::Path::new(path);
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if created_in(&text).is_some() {
            already += 1;
            continue;
        }
        // ⚠ The recovery keyed on the STEM, and a memory with no entry is a
        // detection gap rather than a memory with no beginning. Named, never
        // guessed at from an mtime — mtime records a touch and is wrong by a
        // median of 9.9 days across this corpus.
        let Some(at) = recovered
            .get(stem.as_ref())
            .and_then(|v| v["first"].as_str())
        else {
            undated.push(stem.into_owned());
            continue;
        };
        match with_created(&text, at) {
            Ok(out) => {
                println!("  {stem}  {at}");
                wrote += 1;
                if apply {
                    // ⚠ Rename rather than truncate-then-write: the viewer and
                    // every live session read these, and half a file parses as
                    // a corrupt memory rather than as a partial read.
                    memview::atomic::write(path, out.as_bytes())?;
                }
            }
            Err(why) => conflicts.push(format!("{stem}: {why}")),
        }
    }

    println!(
        "\n{} already dated, {wrote} {} , {} with nothing recovered",
        already,
        if apply { "written" } else { "would be written" },
        undated.len()
    );
    if !conflicts.is_empty() {
        println!(
            "\n⚠ {} conflict(s) — the memory's own date differs, left alone:",
            conflicts.len()
        );
        for one in &conflicts {
            println!("    {one}");
        }
    }
    if !undated.is_empty() {
        println!(
            "\n{} memory/memories no transcript dates. A DETECTION gap, not a\n\
             pruning casualty — more mining recovers them, an mtime does not:",
            undated.len()
        );
        for one in undated.iter().take(30) {
            println!("    {one}");
        }
    }
    if !apply && wrote > 0 {
        println!(
            "\n(nothing written; --apply writes them, and that is the memory session's to run)"
        );
    }
    Ok(())
}
