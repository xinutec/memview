//! Does every memory this commit adds to the index say what its line is for?
//!
//!     git show :MEMORY.md | memory-unjudged --before <(git show HEAD:MEMORY.md)
//!
//! `unjudged-role` in memory-lint finds the same thing over the whole corpus, but
//! as an error it would fail the nightly or the memory's last writer, never the
//! session that added the line (memview#1537). Run by the memory repo's
//! pre-commit hook, this names that session's own lines while it is there to fix
//! them.
//!
//! Reads the staged index on stdin and runs no git: a hook exports `GIT_DIR` to
//! every child (`reference_a_git_hook_exports_its_repo_to_every_child`).

use std::io::Read;

use anyhow::{Context, Result};
use clap::Parser;
use memview::store::Corpus;

/// Name the index lines the staged MEMORY.md (on stdin) adds whose memory has no role.
#[derive(Parser)]
struct Cli {
    /// The index as it was before this change; absent for a new index.
    #[arg(long)]
    before: Option<std::path::PathBuf>,
    /// The memory directory; the user's by default.
    #[arg(long)]
    dir: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut after = String::new();
    std::io::stdin().read_to_string(&mut after)?;
    let before = match &cli.before {
        Some(path) => {
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?
        }
        None => String::new(),
    };
    let dir = cli
        .dir
        .unwrap_or_else(|| reader::home::memory_dir().to_string_lossy().into_owned());
    let corpus = Corpus::load(&dir)?;
    // No record is an empty one: a missing file must not excuse a line. One that
    // does not parse is an error, not an empty record.
    let record = reader::home::file("memory-roles.json");
    let roles: serde_json::Value = match std::fs::read_to_string(&record) {
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("{} does not parse", record.display()))?,
        Err(_) => serde_json::Value::Null,
    };

    let unjudged = memview::lint::newly_unjudged(&before, &after, &corpus, &roles);
    for name in &unjudged {
        println!("{name}");
    }
    if unjudged.is_empty() {
        Ok(())
    } else {
        std::process::exit(1)
    }
}
