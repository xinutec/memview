//! The Python tree over the history's programs: how many read, whether the
//! round-trip law holds for them, and what stopped the rest — ranked, because
//! that list is what to build next.
//!
//!     cargo run --release -p reader --bin python-syntax-report -- <corpus.jsonl> [--show <label> <n>]
//!
//! A law failure is never the queue: it is a defect in the parser or the printer,
//! and `--show` prints the programs behind any label.

use std::collections::{BTreeMap, BTreeSet};

use clap::Parser;

use reader::shell_ops::Op;
use reader::syntax::python::{Outcome, check};

/// The Python tree over the history's programs: how many read, whether the
/// round-trip law holds for them, and what stopped the rest.
#[derive(Parser)]
struct Cli {
    /// The command corpus, from `bash-corpus`.
    corpus: String,
    /// Print the first N programs behind one label.
    #[arg(long, num_args = 2, value_names = ["LABEL", "N"])]
    show: Option<Vec<String>>,
}

/// `--show <label> <n>`, with the count typed: a bad one is refused, not
/// dropped.
fn label_and_count(show: Option<Vec<String>>) -> anyhow::Result<Option<(String, usize)>> {
    let Some([label, n]) = show.as_deref() else {
        return Ok(None);
    };
    let n = n
        .parse()
        .map_err(|e| anyhow::anyhow!("--show {label} {n}: not a count ({e})"))?;
    Ok(Some((label.clone(), n)))
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let path = &cli.corpus;
    let show = label_and_count(cli.show.clone())?;
    let home = std::env::var("HOME").unwrap_or_default();

    let mut programs = BTreeSet::new();
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
            if let Op::Python { source } = op {
                programs.insert(source);
            }
        }
    }

    let mut outcomes: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut reasons: BTreeMap<String, usize> = BTreeMap::new();
    let mut bytes_read = 0usize;
    let mut bytes = 0usize;
    let mut shown = 0usize;
    for program in &programs {
        bytes += program.len();
        let outcome = check(program);
        *outcomes.entry(outcome.label()).or_insert(0) += 1;
        let label = match &outcome {
            Outcome::Refused(refusal) => {
                let label = refusal.reason.label();
                *reasons.entry(label.clone()).or_insert(0) += 1;
                label
            }
            other => {
                if other.holds() {
                    bytes_read += program.len();
                }
                other.label().to_string()
            }
        };
        if let Some((wanted, n)) = &show
            && label.contains(wanted.as_str())
            && shown < *n
        {
            shown += 1;
            println!("--- {label}:\n{program}");
            match &outcome {
                Outcome::Unreadable { printed, refusal } => {
                    println!("--- printed, refused {:?}:\n{printed}", refusal.reason)
                }
                Outcome::TreeDiffers { printed } => println!("--- printed:\n{printed}"),
                Outcome::NotFixpoint { second, third } => {
                    println!("--- second:\n{second}--- third:\n{third}")
                }
                _ => {}
            }
            println!();
        }
    }

    let total = programs.len();
    println!("python programs          {total}");
    for (label, n) in &outcomes {
        println!(
            "  {label:<22} {n:6}  ({:.1}%)",
            100.0 * *n as f64 / total.max(1) as f64
        );
    }
    println!(
        "  bytes read              {:.1}%",
        100.0 * bytes_read as f64 / bytes.max(1) as f64
    );
    println!("refused, by construct:");
    let mut ranked: Vec<(String, usize)> = reasons.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    for (label, n) in ranked.iter().take(30) {
        println!("  {n:6}  {label}");
    }
    Ok(())
}
