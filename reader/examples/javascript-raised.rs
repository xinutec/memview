//! The JavaScript programs the reader says never ran, handed out to be checked.
//!
//!     cargo run --release -p reader --example javascript-raised -- <corpus.jsonl> [--all]
//!
//! The JavaScript twin of `python-raised`, and it exists for the same reason:
//! [`reader::javascript::did_not_run`] claims a program was never accepted by a
//! runtime, and on that claim the reader **discards** every file operation the
//! program named. That is the one direction where being wrong destroys real
//! knowledge rather than inventing some, so the claim wants an outside verdict.
//!
//! This crate must not spawn a process — see `reader/src/lib.rs` — so the
//! sources go out NUL-separated and whoever wants the verdict asks a real
//! runtime, exactly as `nested-why` hands scripts to `bash -n`. Node checks a
//! file rather than a string, and this corpus writes both module and script
//! syntax, so a program counts as accepted if EITHER form parses:
//!
//! ```sh
//! cargo run --release -p reader --example javascript-raised -- <corpus> > /tmp/js.nul
//! python3 - <<'PY'
//! import subprocess, pathlib, tempfile
//! bad = ok = 0
//! for src in pathlib.Path('/tmp/js.nul').read_bytes().split(b'\0'):
//!     if not src.strip(): continue
//!     verdicts = []
//!     for ext in ('.mjs', '.cjs'):
//!         with tempfile.NamedTemporaryFile(suffix=ext, delete=False) as f:
//!             f.write(src)
//!         verdicts.append(subprocess.run(['node', '--check', f.name],
//!                                        capture_output=True).returncode)
//!     ok, bad = (ok + 1, bad) if 0 in verdicts else (ok, bad + 1)
//! print('node refuses:', bad, ' node accepts:', ok)
//! PY
//! ```
//!
//! **Answered 2026-08-22 over `union.jsonl`, against node 24.18.0** — this
//! repository's own pinned node, not an ambient one.
//!
//! - **The reader discards 2 distinct programs, and node refuses both.** No
//!   false positives: nothing is thrown away that could have run.
//! - Of the 1,025 it keeps, node accepts **1,016**. The nine it refuses are
//!   three kinds, and only one is a gap:
//!   - **7 are TypeScript** — `new Map<string, string[]>()`, `(n: number)` —
//!     run by `tsx`, which accepts them. Node is the wrong instrument for those,
//!     and the annotations land in `stray` by design.
//!   - **1 holds an unexpanded shell variable**, so the shell substituted a
//!     value before any runtime saw the text. Two rules meeting, not a defect —
//!     the same finding `python-raised` reports for 19 of its programs.
//!   - **1 is genuinely broken**: a template literal cut short, which is a
//!     heredoc that lost its tail. Its file operations are still counted, and
//!     catching it would mean validating JavaScript syntax, which
//!     `javascript.pest` declines to do by design.
//!
//! ⚠ **The instrument has a known blind spot, and it is not a small one.** Node
//! cannot check TypeScript, so `--all` will always show `tsx` programs as
//! refused. Read the nine, do not just count them.
//!
//! ⚠ **Run it both ways after touching `did_not_run`.** Without `--all` this
//! prints what the reader discards, and node says whether any of it should have
//! been kept — a single acceptance there is a defect, not a tuning question.
//! With `--all` it prints everything, and node says what the reader *kept* that
//! could never have run, which is the cheaper kind of wrong.

use std::collections::BTreeSet;

use reader::shell_ops::Op;
use reader::{javascript, shell_files};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: javascript-raised <corpus.jsonl> [--all]");
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
            let Op::JavaScript { source } = op else {
                continue;
            };
            if everything || javascript::did_not_run(source).is_some() {
                occurrences += 1;
                seen.insert(source.clone());
            }
        }
    }

    eprintln!(
        "{occurrences} calls, {} distinct programs the reader says never ran",
        seen.len()
    );
    for source in &seen {
        print!("{source}\0");
    }
    Ok(())
}
