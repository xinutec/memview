//! Which of the corpus's in-place sed scripts the evaluator follows, and what
//! stops the rest — a census of the script parser alone, with no file text.
//!
//!     cargo run --release -p reader --example sed-scripts -- /tmp/sed-scripts.jsonl
//!
//! Each row is `{"extended": bool, "script": "…"}`; `--show <why> <n>` prints
//! the scripts behind one reason.

use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: sed-scripts <scripts.jsonl> [--show <why> <n>]");
    };
    let show = args.iter().position(|a| a == "--show").and_then(|at| {
        Some((
            args.get(at + 1)?.clone(),
            args.get(at + 2)?.parse::<usize>().ok()?,
        ))
    });
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
