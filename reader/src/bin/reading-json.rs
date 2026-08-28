//! The corpus survey as a small JSON artefact, for the apps to draw.
//!
//!     cargo run --release -p reader --bin reading-json -- <corpus.jsonl> [out.json]
//!
//! Defaults to `~/.claude/corpus/union.jsonl` → `~/.claude/memview/reading.json`,
//! which
//! is what the nightly runs.
//!
//! ⚠ **Mined rather than computed per request, and the reason is a measurement:
//! the survey takes 13 seconds over 146k commands.** That is fine for a report
//! somebody waits on and wrong for a page — it would block a request the whole
//! time and do it again for the next viewer. The artefact it produces is ~6 kB,
//! so both servers hold it in memory and serve it in microseconds.
//!
//! ⚠ **It carries counts and command NAMES, never a command line.** The busiest
//! paths and the unread command names are in it; the text that named them is
//! not. `effects.json` is where verbatim command text lives, and the split is
//! deliberate — this file is small enough to be embedded in a page, so it is
//! held to what a page may safely carry.

use reader::reading::Reading;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let home = std::env::var("HOME").unwrap_or_default();
    let corpus = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| format!("{home}/.claude/corpus/union.jsonl"));
    let out = args.get(2).cloned().unwrap_or_else(|| {
        reader::home::file("reading.json")
            .to_string_lossy()
            .into_owned()
    });

    // Epoch seconds, NOT a formatted string. `reader` is a leaf crate — see
    // `reader/tests/leaf.rs` — and carrying `chrono` so that one field arrives
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

    // ⚠ **Write then rename.** A server reading this file holds it against an
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
