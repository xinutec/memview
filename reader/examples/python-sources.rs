//! Every Python program the corpus runs, one JSON string per line, as the reader
//! finds them — the input for measuring the language the fleet actually writes.
//!
//!     cargo run --release -p reader --example python-sources -- <corpus.jsonl> > programs.jsonl

use reader::shell_ops::Op;

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: python-sources <corpus.jsonl>"))?;
    let home = std::env::var("HOME").unwrap_or_default();
    let mut seen = std::collections::BTreeSet::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let Ok(parsed) = reader::project::read(cmd) else {
            continue;
        };
        let found = reader::shell_files::extract_knowing(&parsed, row["cwd"].as_str(), &home, &[]);
        for op in found.ops {
            if let Op::Python { source } = op
                && seen.insert(source.clone())
            {
                println!("{}", serde_json::to_string(&source)?);
            }
        }
    }
    Ok(())
}
