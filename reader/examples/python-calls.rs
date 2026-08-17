//! Every shell command that invokes Python, and what the reader made of it.
//!
//!     cargo run --release -p reader --example python-calls -- <corpus.jsonl> [--show <n>]
//!
//! `python-report` counts the programs we *found*; `tree-sitter-python-probe`
//! asks whether what we found parses. Neither answers the question in front of
//! both: **is there Python we never noticed was Python?** A call the verb table
//! does not recognise produces no `Op::Python`, so it is absent from every
//! report rather than wrong in one — the failure mode a coverage figure cannot
//! show, because the denominator is what we found.
//!
//! So this reads the argv instead of the verb table. Any word whose basename
//! looks like a Python interpreter makes the command a candidate, and the
//! candidate is then checked against what the chain actually produced. The
//! three honest outcomes:
//!
//! - **read** — an `Op::Python`, the program is in hand;
//! - **a script file** — `python3 scripts/load.py`, correctly not read, because
//!   what that file held then is not in the transcript;
//! - **missed** — neither, and the interpreter word is right there in the argv.
//!
//! ⚠ **A module run is not a miss.** `python3 -m json.tool` carries no program
//! and names no file; it is Python invoked to do something we can see from the
//! argv alone, and counting it as unread would invent a gap.
//!
//! **Answered, 2026-08-17, over `union.jsonl`: 17,453 commands name a Python
//! interpreter and none is missed.** 14,423 are read; 1,908 name a script file
//! whose contents are on disk and not in the transcript; 831 name the
//! interpreter without running it here; 226 are module runs; 49 name a script
//! through a variable nobody can resolve; 12 ask the version; 4 are fed a
//! program on a pipe that the transcript never recorded.
//!
//! The one gap it found was `python3.12`, which the verb table matched as
//! neither of its two literal spellings — see `shell_ops::is_python`. Small, and
//! invisible by construction, which is the argument for owning an instrument
//! that asks the question from outside the table.

use std::collections::BTreeMap;

use reader::shell_ops;
use reader::shell_ops::Op;

/// Whether a word names a Python interpreter, by its basename.
///
/// ⚠ **Deliberately looser than [`shell_ops::is_python`], and it must stay
/// that way.** This is the instrument; the table is what it measures. If the
/// probe asked the table's own question it could never find a spelling the
/// table has not been taught, which is the one thing it exists to find. So it
/// matches `python313` too, and the outcome column is where that word is shown
/// to be a nixpkgs attribute rather than a call.
fn is_interpreter(word: &str) -> bool {
    // `PY=/nix/store/…/bin/python3` names an interpreter and runs nothing.
    if word
        .split('/')
        .next()
        .is_some_and(|head| head.contains('='))
    {
        return false;
    }
    let base = word.rsplit('/').next().unwrap_or(word);
    let stem = base.strip_suffix(".exe").unwrap_or(base);
    if let Some(rest) = stem.strip_prefix("python") {
        // `python`, `python3`, `python3.12` — but not `python-dotenv`.
        return rest.is_empty() || rest.chars().all(|c| c.is_ascii_digit() || c == '.');
    }
    matches!(stem, "pypy" | "pypy3" | "ipython" | "ipython3")
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: python-calls <corpus.jsonl> [--show <n>]");
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(12);
    let home = std::env::var("HOME").unwrap_or_default();

    let mut candidates = 0usize;
    // By the interpreter as it was spelled, so a gap names the spelling that
    // caused it rather than a total.
    let mut outcome: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    let mut missed: Vec<String> = Vec::new();

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
        for simple in &parsed.commands {
            // A wrapper puts the interpreter later in the argv — `nix develop -c
            // python3 -c …` — so the search is over every word, not the first.
            let Some(spelling) = simple.argv.iter().find(|word| is_interpreter(word)) else {
                continue;
            };
            candidates += 1;
            let op = shell_ops::classify(&simple.argv, &simple.heredocs, cwd, &home);
            let flag = |want: &str| simple.argv.iter().any(|word| word == want);
            let verdict = match &op {
                Op::Python { .. } => "read",
                Op::Run { script } if script.ends_with(".py") => "a script file",
                // The interpreter is named but is not what this command runs:
                // `nix develop -c python3 …`, where the inner command is its own
                // `Simple` and is counted there, or `nix-shell -p python313`,
                // where the word is a package and nothing runs at all.
                _ if simple
                    .argv
                    .first()
                    .is_some_and(|first| !is_interpreter(first)) =>
                {
                    "named, not run here"
                }
                // Carries no program and names no file, so there is nothing to
                // read and calling it unread would invent a gap.
                _ if flag("-m") => "a module run",
                _ if flag("-V") || flag("--version") => "a version check",
                // ⚠ **`python -` reads its program from stdin**, and where that
                // stdin is a pipe rather than a heredoc the program is not in
                // the command text at all. Nothing can be read here, and the
                // distinction from a miss is the whole point: one is a gap in
                // the reader, the other is a gap in the evidence.
                _ if flag("-") && simple.heredocs.is_empty() => "a program piped in, unrecorded",
                // `python3 $S/gen.py` — a script we cannot name, which is the
                // undetermined subject the reader records rather than guesses.
                _ if simple
                    .argv
                    .iter()
                    .skip(1)
                    .any(|word| !word.starts_with('-') && word.contains('$')) =>
                {
                    "a script file we cannot name"
                }
                _ => "missed",
            };
            if verdict == "missed" && missed.len() < show {
                missed.push(simple.argv.join(" ").chars().take(120).collect());
            }
            let base = spelling.rsplit('/').next().unwrap_or(spelling).to_string();
            *outcome.entry((base, verdict)).or_default() += 1;
        }
    }

    println!("{candidates} commands name a Python interpreter\n");
    let mut ranked: Vec<_> = outcome.into_iter().collect();
    ranked.sort_by_key(|((spelling, _), n)| (spelling.clone(), std::cmp::Reverse(*n)));
    for ((spelling, verdict), n) in &ranked {
        println!("  {n:>6}  {spelling:<14} {verdict}");
    }

    println!("\nmissed, verbatim:");
    for one in &missed {
        println!("  {one}");
    }
    Ok(())
}
