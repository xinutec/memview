//! Mine the session transcripts for memories used together.
//!
//!     cargo run --release --bin couse [-- <transcript dir>]
//!
//! Writes `couse.json` beside the transcripts — deliberately NOT inside the
//! memory directory, which `scripts/sync.sh` pushes wholesale to a server. The
//! artefact holds only names and counts, but it describes working patterns, and
//! the corpus directory is for memories.
//!
//! Release build on purpose: this reads every byte of every transcript, which is
//! gigabytes.
use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::Result;
use memview::couse;
use memview::store::Corpus;

fn main() -> Result<()> {
    let home = std::env::var("HOME").unwrap_or_default();
    let dir: PathBuf = std::env::args()
        .nth(1)
        .unwrap_or_else(|| format!("{home}/.claude/projects/-Users-pippijn-Code"))
        .into();

    // Filtered against the live corpus, and this is not a formality. The first
    // run of this ranked memview's own test fixtures at the very top —
    // project_alpha, feedback_gamma and reference_beta co-occur in every run of
    // the test suite, which is a fact about the harness, not about the memory.
    let corpus = Corpus::load(dir.join("memory"))?;
    let names: BTreeSet<String> = corpus.docs.keys().cloned().collect();
    println!("{} memories, scanning {}", names.len(), dir.display());

    // Where projects live, so a mention can be attributed to the work it was
    // consulted for. Overridable so the miner is not welded to one machine's
    // layout, and so nothing publishes a home directory.
    let code_root = std::env::var("CODE_ROOT").unwrap_or_else(|_| format!("{home}/Code"));
    let generated = couse::stamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    );

    let found = couse::scan(&dir, &names, &code_root, &generated)?;
    let out = dir.join("couse.json");
    found.save(&out)?;

    println!(
        "{} turns used two or more memories; {} pairs seen in >= {} sessions",
        found.turns,
        found.pairs.len(),
        couse::MIN_SESSIONS
    );
    for pair in found.pairs.iter().take(15) {
        println!(
            "  {:5.2}  {} sessions, {:3} turns  {}  ~  {}",
            pair.npmi, pair.sessions, pair.turns, pair.a, pair.b
        );
    }
    println!("\nwrote {}", out.display());
    Ok(())
}
