//! What a real Python parser makes of the Python our reader accepts.
//!
//!     cargo run --release --example tree-sitter-python-probe -- /tmp/bash-corpus.jsonl [--show <n>]
//!
//! **The comparison here is not the one the shell probe makes, because neither
//! parser can fail.** `python.pest` has a `stray` rule that accepts punctuation
//! it has no reading for, so a program is never rejected whole; `tree-sitter`
//! is built for editors and emits ERROR nodes rather than refusing. So there is
//! no pair of parse rates to set against each other.
//!
//! What there is instead is the question a permissive grammar cannot answer
//! about itself: **how much of what we accepted is not actually Python?** A
//! program our reader misread and drew paths from is indistinguishable, from the
//! inside, from one it read correctly. tree-sitter is the outside opinion — and
//! where it reports an error, the extraction is suspect and every file operation
//! we took from that program is worth doubting.
//!
//! The programs come from the same path the miner uses — the shell reader's
//! `Op::Python`, which covers `python -c`, an interpreter fed by a heredoc, and
//! anything inside a nested `bash -c` or `nix ... -c` — so this measures the
//! Python the fleet actually runs, not a file on disk somewhere.
//!
//! **Answered, 2026-08-06: the grammar stays, and the probe found a phantom.**
//! 9,006 programs; `tree-sitter-python` 0.25.0 reads 8,903 (98.9%) and rejects
//! 103 (1.1%). Our exposure to those is **39 of 10,134 file operations, 0.4%**.
//!
//! What the rejects are, which is the interesting part:
//! - **`$VAR` left literal (17).** By design — there is no value to expand it to
//!   — so a Python parser calling it a syntax error is the two rules meeting, not
//!   a defect.
//! - **Escaped quotes inside an f-string (~40)**, `f"{d[\"key\"]}"`. Written that
//!   way by the agent, inside single shell quotes, so the backslashes reached the
//!   interpreter. Checked against the real thing rather than assumed: it is a
//!   `SyntaxError: unexpected character after line continuation character` on
//!   **Python 3.12.13**, PEP 701 notwithstanding. **Those commands never ran** —
//!   so every file operation recorded from one is work that did not happen.
//!
//! That is the finding worth keeping: not that a different parser would read
//! more, but that ours reads programs that could not execute and records their
//! file operations as fact. 0.4% is too little to put a C toolchain in the
//! product for — and the transcripts already carry the cheaper answer, since a
//! failed command says so in its result and `doing::Verdict` already reads it.

use std::collections::BTreeMap;

use reader::shell_ops::Op;
use reader::{python, shell_files};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: tree-sitter-python-probe <corpus.jsonl> [--show <n>]");
    };
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(6);
    let home = std::env::var("HOME").unwrap_or_default();

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into())?;

    let text = std::fs::read_to_string(path)?;
    let mut programs = 0usize;
    let mut clean = 0usize;
    // Programs tree-sitter rejects, and — the number that decides whether this
    // matters — how many file operations we drew out of exactly those.
    let mut suspect = 0usize;
    let mut suspect_uses = 0usize;
    let mut total_uses = 0usize;
    let mut by_error: BTreeMap<String, usize> = BTreeMap::new();
    let mut examples: Vec<String> = Vec::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        let Ok(parsed) = reader::project::read(cmd) else {
            continue;
        };
        for op in &shell_files::extract(&parsed, cwd, &home).ops {
            let Op::Python { source } = op else { continue };
            programs += 1;
            let uses = python::read(source).uses.len();
            total_uses += uses;

            let Some(tree) = parser.parse(source.as_bytes(), None) else {
                continue;
            };
            if !tree.root_node().has_error() {
                clean += 1;
                continue;
            }
            suspect += 1;
            suspect_uses += uses;
            // The text under the first error names the construct far better than
            // the node kind does — `str` and `identifier` are most of any tree.
            if let Some(at) = first_error(tree.root_node()) {
                let bytes = &source.as_bytes()[at.byte_range()];
                let head: String = String::from_utf8_lossy(bytes).chars().take(40).collect();
                *by_error.entry(head.replace('\n', "⏎")).or_default() += 1;
            }
            if examples.len() < show {
                examples.push(source.trim().chars().take(300).collect());
            }
        }
    }

    println!("python programs        {programs}");
    println!(
        "  a real parser reads  {clean}  ({:.1}%)",
        100.0 * clean as f64 / programs.max(1) as f64
    );
    println!(
        "  it reports an error  {suspect}  ({:.1}%)",
        100.0 * suspect as f64 / programs.max(1) as f64
    );
    println!("file operations we took {total_uses}");
    println!(
        "  from a suspect program {suspect_uses}  ({:.1}%) — these are the ones worth doubting",
        100.0 * suspect_uses as f64 / total_uses.max(1) as f64
    );

    println!("\nwhat the first error sits on:");
    let mut ranked: Vec<_> = by_error.into_iter().collect();
    ranked.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    for (head, n) in ranked.into_iter().take(15) {
        println!("  {n:>5}  {head:?}");
    }
    println!("\nprograms it rejects:");
    for program in &examples {
        println!("─────\n{program}");
    }
    Ok(())
}

/// The first ERROR or MISSING node, depth first.
fn first_error(node: tree_sitter::Node) -> Option<tree_sitter::Node> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(|child| child.has_error())
        .find_map(first_error)
}
