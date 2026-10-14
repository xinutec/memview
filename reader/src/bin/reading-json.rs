//! The corpus survey as a small JSON artefact, for the apps to draw.
//!
//!     cargo run --release -p reader --bin reading-json -- <corpus.jsonl> [out.json]
//!
//! Defaults to `~/.claude/memview/cache/bash-corpus.jsonl` →
//! `~/.claude/memview/cache/reading.json`, which is what the nightly runs.
//!
//! Mined, not computed per request: the survey takes seconds, the artefact is a
//! few kB.
//!
//! It carries counts and command NAMES, never a command line. The busiest
//! paths and the unread command names are in it; the text that named them is
//! not. `effects.json` is where verbatim command text lives, and the split is
//! deliberate — this file is small enough to be embedded in a page, so it is
//! held to what a page may safely carry.

use clap::Parser;
use reader::reading::Reading;

/// The corpus survey as a small JSON artefact, for the apps to draw.
#[derive(Parser)]
struct Cli {
    /// The command corpus [default: bash-corpus.jsonl in the cache].
    corpus: Option<String>,
    /// Where to write [default: reading.json in the cache].
    out: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let home = std::env::var("HOME").unwrap_or_default();
    let corpus = cli.corpus.unwrap_or_else(|| {
        reader::home::cache("bash-corpus.jsonl")
            .to_string_lossy()
            .into_owned()
    });
    let out = cli.out.unwrap_or_else(|| {
        reader::home::cache("reading.json")
            .to_string_lossy()
            .into_owned()
    });

    // Epoch seconds, NOT a formatted string. `reader` is a leaf crate — see
    // `reader/tests/suite/leaf.rs` — and carrying `chrono` so that one field arrives
    // pre-formatted would put a date library in every binary that links the
    // parser. Both clients are Angular and already have a date pipe.
    let at = std::fs::metadata(&corpus)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);

    let text = std::fs::read_to_string(&corpus)?;
    let read = Reading::of_corpus(&text, &home)?;
    let summary = read.summary(at);
    let json = serde_json::to_vec_pretty(&summary)?;

    // Write then rename. A server reading this file holds it against an
    // mtime; a plain write lets it read half a document and cache the parse
    // failure until the next night.
    let tmp = format!("{out}.tmp");
    std::fs::write(&tmp, &json)?;
    std::fs::rename(&tmp, &out)?;

    println!(
        "{} commands, {:.1}% understood, {} paths → {} ({:.1} kB)",
        summary.commands,
        summary.understood,
        summary.distinct_paths,
        out,
        json.len() as f64 / 1e3,
    );
    Ok(())
}
