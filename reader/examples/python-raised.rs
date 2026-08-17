//! The Python programs the reader says never ran, handed out to be checked.
//!
//!     cargo run --release -p reader --example python-raised -- <corpus.jsonl>
//!
//! [`reader::python::did_not_run`] claims a program raised a `SyntaxError`
//! before its first statement, and on that claim the reader **discards** every
//! file operation the program named. That is the one direction where being
//! wrong destroys real knowledge rather than inventing some, so the claim wants
//! an outside verdict.
//!
//! This crate must not spawn a process — see `reader/src/lib.rs` — so the
//! sources go out NUL-separated and whoever wants the verdict asks a real
//! interpreter, exactly as `nested-why` hands scripts to `bash -n`:
//!
//! ```sh
//! cargo run --release -p reader --example python-raised -- <corpus> \
//!   | python3 -c 'import ast,sys
//! bad = ok = 0
//! for src in sys.stdin.read().split("\0"):
//!     if not src.strip(): continue
//!     try: ast.parse(src); ok += 1
//!     except SyntaxError: bad += 1
//! print("raised:", bad, " parsed fine:", ok)'
//! ```
//!
//! **Answered 2026-08-17 over `union.jsonl`, against CPython 3.12.14** — the
//! version the fleet actually ran, which matters because PEP 701 changed this
//! corner. Of 12,240 distinct programs the reader finds, `ast.parse` refuses
//! **72**; the reader flags **39**, and refuses **none that CPython accepts**.
//!
//! The 33 it does not flag are two kinds, and only one is a gap:
//!
//! - **19 hold an unexpanded shell variable** — `f"{$s}"`, `open('…${p%%:*}…')`.
//!   The shell substituted a value before the interpreter ever saw the text, so
//!   the program that ran was fine and flagging it would delete real work. Two
//!   rules meeting, not a defect.
//! - **14 are genuinely broken** — an unterminated triple-quoted string, an
//!   unclosed bracket, a stray `⚠`; mostly a heredoc whose body was cut short.
//!   Their file operations are still counted, and catching them would mean
//!   validating Python syntax, which `python.pest` declines to do by design.
//!
//! ⚠ **The over-claim direction is the one that costs.** Flagging a program
//! *discards* everything it named, so a false positive destroys knowledge while
//! a false negative only fails to gain any. An earlier, broader rule — any
//! backslash in a replacement field — read two working programs as broken,
//! because 3.12 permits `f"{'\n'.join(x)}"`. Run this both ways after touching
//! [`reader::python::did_not_run`].
//!
//! `tree-sitter-python` is not the authority to use here: it accepts 10 of the
//! programs CPython refuses, being an editor's parser built to keep going.

use std::collections::BTreeSet;

use reader::shell_ops::Op;
use reader::{python, shell_files};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: python-raised <corpus.jsonl> [--all]");
    let everything = args.iter().any(|a| a == "--all");
    let home = std::env::var("HOME").unwrap_or_default();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut occurrences = 0usize;

    for line in std::fs::read_to_string(path)?.lines() {
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
            // ⚠ **Both directions, or the claim is only half-checked.** Without
            // `--all` this prints what the reader discards, and a real parser
            // says whether any of it should have been kept. With `--all` it
            // prints everything, and the same parser says what the reader
            // *kept* that could never have run.
            if everything || python::did_not_run(source).is_some() {
                occurrences += 1;
                seen.insert(source.clone());
            }
        }
    }

    eprintln!(
        "{occurrences} calls, {} distinct programs the reader says raised",
        seen.len()
    );
    for source in &seen {
        print!("{source}\0");
    }
    Ok(())
}
