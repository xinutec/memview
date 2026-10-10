//! The memories that answer a question, best first — for a session that needs
//! one and cannot rely on the index having a line for it.
//!
//!     cargo run --release --bin memory-find -- [--limit N] [--dir D] <words…>
//!
//! The same search memview's own page runs ([`memview::store::Corpus::search`]),
//! so the two cannot rank differently. Each hit prints its path, so the next step
//! is reading the file rather than another lookup.
//!
//! Why it exists (memview#1542): the index is the only reliable way a session
//! finds a memory, and it is capped, so every memory with a line costs another
//! its line. A search that finds a memory without one is what lets a memory live
//! outside the index and still be read.

use anyhow::Result;
use clap::Parser;
use memview::couse::CoUse;
use memview::store::Corpus;

#[derive(Parser)]
struct Cli {
    /// What you are looking for, in your own words — a command's flags
    /// included, so options go before it.
    #[arg(required = true, trailing_var_arg = true, allow_hyphen_values = true)]
    query: Vec<String>,
    /// How many to print.
    #[arg(long, default_value_t = 8)]
    limit: usize,
    /// The memory directory [default: the corpus].
    #[arg(long)]
    dir: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let dir = cli
        .dir
        .unwrap_or_else(|| reader::home::memory_dir().to_string_lossy().into_owned());
    let corpus = Corpus::load(&dir)?;
    // The co-use prior, when it has been mined. Absent changes only the order
    // among comparable answers.
    let usage = std::path::Path::new(&dir)
        .parent()
        .map(|p| p.join("couse.json"))
        .and_then(|p| CoUse::load(&p))
        .map(|c| c.usage)
        .unwrap_or_default();
    let query = cli.query.join(" ");
    let found = corpus.search(&query, &usage);
    if found.hits.is_empty() {
        println!("nothing matches {query:?}");
        return Ok(());
    }
    if found.relaxed {
        println!("no memory holds every word; these hold some of them");
    }
    for hit in found.hits.iter().take(cli.limit) {
        let cue = hit
            .meta
            .teaser
            .as_deref()
            .unwrap_or(hit.meta.description.as_str());
        println!("{}/{}.md", dir.trim_end_matches('/'), hit.meta.name);
        println!("    {}", one_line(cue, 150));
    }
    let more = found.hits.len().saturating_sub(cli.limit);
    if more > 0 {
        println!("… {more} more (--limit)");
    }
    Ok(())
}

/// The cue on one line, cut at a character boundary.
fn one_line(text: &str, most: usize) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match flat.char_indices().nth(most) {
        Some((at, _)) => format!("{}…", &flat[..at]),
        None => flat,
    }
}
