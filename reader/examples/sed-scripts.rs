//! Which of the corpus's in-place sed scripts the evaluator follows, and what
//! stops the rest — a census of the script parser alone, with no file text.
//!
//!     cargo run --release -p reader --example sed-scripts -- /tmp/sed-scripts.jsonl
//!
//! Each row is `{"extended": bool, "script": "…"}`; `--show <why> <n>` prints
//! the scripts behind one reason.

use std::collections::BTreeMap;

use clap::Parser;

/// Which of the corpus's in-place sed scripts the evaluator follows, and what
/// stops the rest.
#[derive(Parser)]
struct Cli {
    /// The sed scripts, one JSON row each.
    scripts: String,
    /// Print the first N scripts behind one reason.
    #[arg(long, num_args = 2, value_names = ["WHY", "N"])]
    show: Option<Vec<String>>,
}

/// `--show <why> <n>`, with the count typed: a bad one is refused, not
/// dropped.
fn label_and_count(show: Option<Vec<String>>) -> anyhow::Result<Option<(String, usize)>> {
    let Some([why, n]) = show.as_deref() else {
        return Ok(None);
    };
    let n = n
        .parse()
        .map_err(|e| anyhow::anyhow!("--show {why} {n}: not a count ({e})"))?;
    Ok(Some((why.clone(), n)))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let path = &cli.scripts;
    let show = label_and_count(cli.show.clone())?;
    let (mut total, mut followed, mut shown) = (0usize, 0usize, 0usize);
    let mut why: BTreeMap<String, usize> = BTreeMap::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let row: serde_json::Value = serde_json::from_str(line)?;
        let (Some(script), Some(extended)) = (row["script"].as_str(), row["extended"].as_bool())
        else {
            continue;
        };
        total += 1;
        match reader::predict::sed::apply(&[script], extended, "") {
            Ok(_) => followed += 1,
            Err(name) => {
                if let Some((wanted, n)) = &show
                    && name.contains(wanted.as_str())
                    && shown < *n
                {
                    shown += 1;
                    println!("--- {name}:\n{script}\n");
                }
                *why.entry(name).or_insert(0) += 1;
            }
        }
    }
    println!("in-place scripts   {total}");
    println!(
        "  followed         {followed}  ({:.1}%)",
        100.0 * followed as f64 / total.max(1) as f64
    );
    println!("  refused, by why:");
    let mut ranked: Vec<(String, usize)> = why.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (name, n) in ranked.iter().take(30) {
        println!("  {n:7}  {name}");
    }
    Ok(())
}
