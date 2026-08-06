//! What the fleet's commands carry that nothing looks inside.
//!
//!     cargo run --release --bin opacity -- <corpus.jsonl> [--show <n>]
//!
//! The fourth of the family after `shell-report` (the grammar), `shell-files`
//! (the shell's semantics) and `python-report` (the Python inside it). Those
//! three answer *how much do we read*; this one answers **where what is left
//! actually is**, which is the question that decides what to build next.
//!
//! ⚠ **Two kinds of opacity, and only the first has ever had a number.** A
//! command not in the table is counted already — 13,448 of them, `$ADB` and
//! `tsx` and `vitest` at the top. But a command we *do* understand still hands
//! us text nobody reads: the script `sed` is given, the pattern `grep` is looking
//! for, the body of a heredoc that is not Python, a word we refuse to resolve
//! because it holds a `$`. None of that has ever been measured, and it is the
//! opacity the goal is about.
//!
//! **The report is the method, not a status line** — the same rule the grammar
//! was grown by. Instinct said the next reader should be JavaScript; the numbers
//! said `tsx`/`vitest`/`playwright` are 4,201 calls of plain file operands while
//! `node -e` is 724 calls with 23 writes. Rank first, then build.
//!
//! ⚠ **The language of a heredoc body is SNIFFED, and a sniff is a guess.** It
//! is reported as "looks like" for that reason. The point is the ranking — is
//! there a kilobyte of SQL here or a megabyte — not the label on any one body.

use std::collections::BTreeMap;

use memview::shell_ops::Op;
use memview::{shell, shell_files};

/// A tally of things and how many bytes they came to.
#[derive(Default)]
struct Weighed {
    calls: usize,
    bytes: usize,
    /// A few verbatim, because a count names a size and never a construct.
    seen: Vec<String>,
}

impl Weighed {
    fn add(&mut self, text: &str, keep: usize) {
        self.calls += 1;
        self.bytes += text.len();
        if self.seen.len() < keep {
            self.seen.push(cut(text));
        }
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: opacity <corpus.jsonl> [--show <n>]");
    };
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(12);
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;
    let mut calls = 0usize;
    let mut unread: BTreeMap<String, usize> = BTreeMap::new();
    // The payloads, by what the command meant to do with them.
    let mut kinds: BTreeMap<&'static str, Weighed> = BTreeMap::new();
    // Heredoc bodies by the language they look like.
    let mut bodies: BTreeMap<&'static str, Weighed> = BTreeMap::new();
    // Words we refuse to resolve, by why.
    let mut unresolved: BTreeMap<&'static str, Weighed> = BTreeMap::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        calls += 1;
        let Ok(parsed) = shell::parse(cmd) else {
            continue;
        };

        // Heredoc bodies and unresolvable words come off the parse, because they
        // are properties of the text as written rather than of what it does.
        for simple in &parsed {
            for body in &simple.heredocs {
                bodies.entry(looks_like(body)).or_default().add(body, show);
            }
            for word in &simple.argv {
                if let Some(why) = refuses(word) {
                    unresolved.entry(why).or_default().add(word, show);
                }
            }
        }

        let found = shell_files::extract(&parsed, cwd, &home);
        for (name, n) in found.unhandled {
            *unread.entry(name).or_default() += n;
        }
        // ⚠ `ops` already contains what the nested and remote shells did — they
        // are absorbed into the list rather than left as one opaque word — so
        // this counts the payloads inside a `nix develop -c` too.
        for op in &found.ops {
            match op {
                // A regex is a program. We keep it whole and read none of it, so
                // `grep -c "ingested report"` and `grep '^src/[a-z]*\.ts$'` are
                // the same to us: the second names a shape of file and the first
                // does not, and nothing can tell them apart.
                Op::Search { pattern, .. } => kinds
                    .entry("search pattern (regex)")
                    .or_default()
                    .add(pattern, show),
                // The sed/awk/jq script. It says what the rewrite DOES, which is
                // the difference between a pager and an edit.
                Op::Transform {
                    program, in_place, ..
                } => kinds
                    .entry(if *in_place {
                        "transform program, rewriting"
                    } else {
                        "transform program, reading"
                    })
                    .or_default()
                    .add(program, show),
                // A script named but never opened. Its contents are on disk, not
                // in the transcript, so this is opacity of a different kind — and
                // worth separating for that reason.
                Op::Run { script } => kinds
                    .entry("script file, unopened")
                    .or_default()
                    .add(script, show),
                _ => {}
            }
        }
    }

    println!("{calls} Bash calls\n");
    println!("── commands not in the table ──");
    ranked(&unread, show);

    println!("\n── payload we carry and never read ──");
    for (kind, weighed) in by_bytes(&kinds) {
        println!(
            "  {:>6} calls  {:>9}  {kind}",
            weighed.calls,
            bytes(weighed.bytes)
        );
        for one in weighed.seen.iter().take(3) {
            println!("           | {one}");
        }
    }

    println!("\n── heredoc bodies, by the language they LOOK like (sniffed) ──");
    for (guess, weighed) in by_bytes(&bodies) {
        println!(
            "  {:>6} bodies {:>9}  {guess}",
            weighed.calls,
            bytes(weighed.bytes)
        );
    }

    println!("\n── words we refuse to resolve ──");
    for (why, weighed) in by_bytes(&unresolved) {
        println!("  {:>6} words  {why}", weighed.calls);
        for one in weighed.seen.iter().take(3) {
            println!("           | {one}");
        }
    }
    Ok(())
}

/// Why a word cannot become a value, or `None` if it can.
///
/// ⚠ **These are the bottom of the lattice, and the whole point of counting them
/// is that some are permanent and some are not.** A bare `$ADB` can never be
/// resolved — there is no value to expand it to, and inventing one would name a
/// file nobody touched. A literal loop variable is a different matter entirely:
/// 3,078 of the corpus's 10,398 `for` loops iterate a word list that is right
/// there in the text. Lumped together they look like one intractable problem.
fn refuses(word: &str) -> Option<&'static str> {
    if !word.contains('$') {
        return None;
    }
    Some(if word.starts_with("$(") || word.contains("$(") {
        "holds the output of another command"
    } else if word == "$ADB" || (word.starts_with('$') && !word.contains('/')) {
        "a bare variable"
    } else {
        "a path built around a variable"
    })
}

/// What a heredoc body looks like. A guess, reported as one.
///
/// Ordered by how distinctive the mark is rather than by how common the language
/// is: a `SELECT` is unmistakable, an indented line is not.
fn looks_like(body: &str) -> &'static str {
    let head = body.trim_start();
    let upper = body.to_uppercase();
    if head.starts_with("#!") {
        "a script with a shebang"
    } else if head.starts_with('{') || head.starts_with('[') {
        "JSON"
    } else if upper.contains("SELECT ") || upper.contains("CREATE TABLE") {
        "SQL"
    } else if head.starts_with("import ") || head.starts_with("from ") || head.contains("\ndef ") {
        "Python"
    } else if head.starts_with("let ") && body.contains(" in ") {
        "Dhall"
    } else if head.starts_with("---") || head.starts_with("apiVersion:") {
        "YAML"
    } else if head.starts_with('<') {
        "markup"
    } else if body.contains("\n#") || head.starts_with("set -e") || body.contains("$(") {
        "shell"
    } else {
        "prose, or nothing recognised"
    }
}

fn by_bytes<K: Copy + Ord>(all: &BTreeMap<K, Weighed>) -> Vec<(K, &Weighed)> {
    let mut ranked: Vec<(K, &Weighed)> = all.iter().map(|(k, v)| (*k, v)).collect();
    ranked.sort_by_key(|(_, weighed)| std::cmp::Reverse(weighed.bytes));
    ranked
}

fn ranked(all: &BTreeMap<String, usize>, show: usize) {
    let mut sorted: Vec<(&String, &usize)> = all.iter().collect();
    sorted.sort_by_key(|(_, n)| std::cmp::Reverse(**n));
    for (name, n) in sorted.into_iter().take(show) {
        println!("  {n:>6}  {name}");
    }
}

fn bytes(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1e6)
    } else if n >= 1_000 {
        format!("{:.0} kB", n as f64 / 1e3)
    } else {
        format!("{n} B")
    }
}

fn cut(text: &str) -> String {
    let one_line = text.replace('\n', "⏎");
    let mut out: String = one_line.chars().take(90).collect();
    if one_line.chars().count() > 90 {
        out.push('…');
    }
    out
}
