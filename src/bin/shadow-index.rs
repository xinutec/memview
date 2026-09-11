//! The `MEMORY.md` the corpus declares, beside the one a session wrote.
//!
//!     cargo run --release --bin shadow-index
//!     cargo run --release --bin shadow-index -- --write
//!
//! Prints the disagreements; `--write` also drops the assembled file in
//! memview's cache so it can be diffed with ordinary tools.
//!
//! ⚠ **It never touches `MEMORY.md`.** The corpus is not memview's to edit —
//! the tools are built here and the memory session runs them (memview#1310).
//! The output guides; a session reads it and decides.
//!
//! ⚠ **When the two disagree, the ALGORITHM is the first suspect.** This
//! assembles from `teaser:` frontmatter, which most of the corpus does not yet
//! carry, so a difference is far more likely to be a gap in the data than a
//! fault in the file somebody maintains by hand.

use anyhow::{Context, Result};
use memview::store::Corpus;

fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let dir = std::env::var("MEMORY_DIR")
        .unwrap_or_else(|_| format!("{home}/.claude/projects/-Users-pippijn-Code/memory"));
    let write = std::env::args().any(|a| a == "--write");

    let corpus = Corpus::load(&dir)?;
    let shadow = memview::shadow::assemble(&corpus);
    let rendered = memview::shadow::render(&shadow);

    let lines: usize = shadow.sections.iter().map(|s| s.lines.len()).sum();
    let written_links = corpus
        .index_md
        .as_deref()
        .map(|i| memview::store::index_links(i).len())
        .unwrap_or(0);

    println!(
        "assembled {lines} lines in {} sections, {} bytes of {}",
        shadow.sections.len(),
        shadow.bytes,
        memview::lint::INDEX_CEILING
    );
    println!("the written index carries {written_links} links\n");

    // ⚠ **The gaps are printed BEFORE the drift, and that order is deliberate.**
    // A reader who sees "37 lines differ" first will read the artefact as a
    // proposal to rewrite the file. The first thing to know is how much of the
    // corpus this can speak for at all.
    println!(
        "CANNOT BE ASSEMBLED — indexed, but the memory declares no `teaser:` ({})",
        shadow.no_teaser.len()
    );
    println!("  A gap in the DATA, not a proposal to drop. Nothing here is an opinion.");
    for name in shadow.no_teaser.iter().take(10) {
        println!("      {name}");
    }
    if shadow.no_teaser.len() > 10 {
        println!("      … and {} more", shadow.no_teaser.len() - 10);
    }

    println!(
        "\nDECLARED, NOT CARRIED — has a `teaser:`, and the index has no line ({})",
        shadow.declared_not_carried.len()
    );
    println!("  Also not a proposal: a memory may declare a line and still not belong.");
    for name in shadow.declared_not_carried.iter().take(10) {
        println!("      {name}");
    }
    if shadow.declared_not_carried.len() > 10 {
        println!(
            "      … and {} more",
            shadow.declared_not_carried.len() - 10
        );
    }

    // ⚠ **This is the finding the artefact exists for.** The teaser moved into
    // the memory so it could not rot apart from what it describes; these are the
    // places where it has.
    println!(
        "\n⚠ DRIFTED — the memory's own teaser and the written line disagree ({})",
        shadow.drifted.len()
    );
    println!("  Each is one line that has come apart from the memory it describes.");
    for line in shadow.drifted.iter().take(20) {
        println!("      {}", line.name);
        println!(
            "        file says   {}",
            line.written.as_deref().unwrap_or("")
        );
        println!("        memory says {}", line.teaser);
    }
    if shadow.drifted.len() > 20 {
        println!("      … and {} more", shadow.drifted.len() - 20);
    }

    if !shadow.over_ceiling.is_empty() {
        println!(
            "\n⚠ OVER THE CEILING — {} assembled lines fall off the bottom",
            shadow.over_ceiling.len()
        );
        for name in shadow.over_ceiling.iter().take(10) {
            println!("      {name}");
        }
    }

    if write {
        let path = reader::home::cache("MEMORY.shadow.md");
        std::fs::write(&path, &rendered).with_context(|| format!("writing {}", path.display()))?;
        println!("\nwrote {}", path.display());
        println!("  ⚠ a CACHE, not a record — and never MEMORY.md.");
    } else {
        println!("\n(nothing written; --write drops the assembled file in memview's cache)");
    }
    Ok(())
}
