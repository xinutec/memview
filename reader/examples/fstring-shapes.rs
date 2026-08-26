//! What an interpolated f-string still tells you about the path.
//!
//!     cargo run --release -p reader --example fstring-shapes -- <corpus.jsonl> [--show <n>]
//!
//! `python::text` abandons an f-string the moment it meets a `{` — the comment
//! there reads "names a file whose name is not here", which is true of the hole
//! and false of everything around it. `f"data/{name}.stream"` cannot be named,
//! but it is not unknowable either: it is `data/*.stream`, a language with a
//! locus, which is exactly what [`crate::program::Program::bounded`] and
//! `located` already express for globs and literal sets.
//!
//! Before writing that rule, this asks whether the literal text is worth
//! anything. A bare `f"{p}"` renders `*` and buys nothing; `f"{root}/out.json"`
//! has no locus but a real language; `f"logs/{d}/x.log"` has both.
//!
//! ⚠ **This classifies the TEXT of every f-string in every Python program the
//! chain read, NOT only those in a file operation's path argument.** The
//! reader's own traversal is what decides that, and asking it here would mean
//! rebuilding it. So this sizes the SHAPE question — how much literal text do
//! interpolated strings carry — and the exact subset is what the change itself
//! will report as a delta. Read the buckets, not the total.
//!
//! **Answered 2026-08-26 over `union.jsonl`.** 19,848 Python programs hold
//! 7,826 interpolated f-strings, and **94.3% of them are not paths at all** —
//! `*=*`, `* *`, `Bearer *`, `print` formatting. Filtered to those in a file
//! operation's path argument, the population is 289 and the distribution
//! inverts:
//!
//!     21.8%  Nothing    a URL, a log line, `*/*`
//!     33.9%  Language   `*/MEMORY.md`, `*/frames.jsonl` — the NAME is certain
//!     44.3%  Located    `eval/goldens/*.json`, `data/archive/*.stream`
//!
//! So roughly **226 of 289 would gain a real answer**, and reading the
//! unfiltered total instead would have sized the same rule at 2.6%.
//!
//! ⚠ **289 is a LOWER bound on the population, not the population.** This
//! matches an f-string sitting directly inside `open(`/`Path(`/…, so it misses
//! `p = f"…"` followed by `open(p)` entirely — which `scope()` handles and which
//! is common. The reader's own count of the category is 601 (#1142).

use std::collections::BTreeMap;

use reader::shell_ops::{self, Op};

/// What the literal text around the holes is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Worth {
    /// `f"{p}"`, `f"{a}{b}"` — renders `*`, which is every file there is.
    Nothing,
    /// `f"{root}/out.json"`, `f"{stem}.stream"` — no directory is certain, but
    /// the shape is: a language without a locus.
    Language,
    /// `f"logs/{day}.log"` — the directory before the first hole is written
    /// out, so both halves are real.
    Located,
}

/// Render an f-string body as a glob, `{…}` becoming `*`.
///
/// ⚠ **Nesting is not followed and does not need to be.** A format spec
/// (`{x:>3}`) and a nested brace both end at the first `}` for this purpose:
/// everything between the braces is unknown either way, and the point is the
/// text OUTSIDE them.
fn pattern(inner: &str) -> String {
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push('{');
            }
            '{' => {
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                }
                // Adjacent holes are one unknown run, not two.
                if !out.ends_with('*') {
                    out.push('*');
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// ⚠ **"Has literal text" is NOT "is a path", and reading it that way put 94.5%
/// of the corpus's f-strings in the language bucket on the first run.** What
/// was in there was `*=*`, `* *`, `Bearer *` — `print` format strings, whose
/// literal characters are spaces and colons. A path shape has to be asked for.
///
/// ⚠ **This deliberately does NOT call `python::path_shaped`, which is the rule
/// it sized.** Same argument as `python-calls::is_interpreter`: an instrument
/// that asks the implementation's own question can never show a shape the
/// implementation gets wrong. They agree today, and the buckets are printed with
/// samples so a session can see when they stop agreeing.
fn worth(pattern: &str) -> Worth {
    let literal: String = pattern.chars().filter(|c| *c != '*').collect();
    // A space or a tab says formatting, not a filename. So does a URL scheme,
    // which is a subject but not a file on this machine.
    if literal.trim().is_empty() || literal.contains([' ', '\t']) || pattern.contains("://") {
        return Worth::Nothing;
    }
    // Something in it must look like a path or a name: a separator, or a
    // literal extension after the last hole.
    let extension = pattern
        .rsplit('*')
        .next()
        .is_some_and(|tail| tail.starts_with('.') && tail.len() > 1 && !tail.contains('/'));
    if !literal.contains('/') && !extension {
        return Worth::Nothing;
    }
    // ⚠ `*/*` passes every test above and says nothing: a separator alone
    // names neither a directory nor a file. The literal has to carry a
    // character that could be part of a NAME.
    if literal.chars().all(|c| c == '/' || c == '.') {
        return Worth::Nothing;
    }
    match pattern.split('*').next().unwrap_or_default().contains('/') {
        true => Worth::Located,
        false => Worth::Language,
    }
}

/// The file operations whose path argument is the population this is about.
///
/// ⚠ **Names only, and the match is on the text just before the literal.** The
/// reader's traversal is what really decides argument position; this is an
/// approximation of it, and it is here so the buckets can be read against the
/// population that matters instead of against every f-string in the corpus.
const OPENERS: &[&str] = &[
    "open(",
    "Path(",
    "read_text(",
    "write_text(",
    "read_bytes(",
    "write_bytes(",
    "savefig(",
    "to_csv(",
    "save(",
    "remove(",
    "unlink(",
    "makedirs(",
    "mkdir(",
    "exists(",
    "isfile(",
    "isdir(",
    "stat(",
    "rename(",
    "replace(",
    "copy(",
    "copyfile(",
    "load(",
    "dump(",
    "imread(",
    "imwrite(",
    "glob(",
];

/// Whether an f-string starting at `at` sits in a file operation's argument.
fn in_path_position(source: &[char], at: usize) -> bool {
    let from = at.saturating_sub(24);
    let before: String = source[from..at].iter().collect();
    OPENERS.iter().any(|opener| before.ends_with(opener))
}

/// Every f-string literal in a program's text, as its inner body.
///
/// Deliberately lexical: `python.pest` keeps string rules atomic for the same
/// reason, and a scanner that stops at the closing quote of the same kind is
/// what that grammar does too.
fn fstrings(source: &str) -> Vec<(String, bool)> {
    let bytes: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == '\'' || c == '"' {
            // A prefix of up to two letters sits directly before the quote.
            let mut at = i;
            let mut prefix = String::new();
            while at > 0 && bytes[at - 1].is_ascii_alphabetic() && prefix.len() < 2 {
                at -= 1;
                prefix.insert(0, bytes[at].to_ascii_lowercase());
            }
            let triple = bytes.get(i + 1) == Some(&c) && bytes.get(i + 2) == Some(&c);
            let fence = if triple { 3 } else { 1 };
            let mut j = i + fence;
            let mut body = String::new();
            while j < bytes.len() {
                if !triple && (bytes[j] == c || bytes[j] == '\n') {
                    break;
                }
                if triple
                    && bytes[j] == c
                    && bytes.get(j + 1) == Some(&c)
                    && bytes.get(j + 2) == Some(&c)
                {
                    break;
                }
                if bytes[j] == '\\' {
                    j += 2;
                    body.push('x');
                    continue;
                }
                body.push(bytes[j]);
                j += 1;
            }
            if prefix.contains('f') && body.contains('{') {
                let positioned = in_path_position(&bytes, at);
                found.push((body, positioned));
            }
            i = j + fence;
            continue;
        }
        i += 1;
    }
    found
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: fstring-shapes <corpus.jsonl> [--show <n>]");
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(12);
    let home = std::env::var("HOME").unwrap_or_default();

    let mut programs = 0usize;
    let mut total = 0usize;
    let mut in_position = 0usize;
    let mut by_worth: BTreeMap<Worth, usize> = BTreeMap::new();
    let mut positioned_worth: BTreeMap<Worth, usize> = BTreeMap::new();
    let mut positioned_samples: BTreeMap<Worth, BTreeMap<String, usize>> = BTreeMap::new();
    let mut samples: BTreeMap<Worth, BTreeMap<String, usize>> = BTreeMap::new();
    let mut loci: BTreeMap<String, usize> = BTreeMap::new();

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
            let Op::Python { source } =
                shell_ops::classify(&simple.argv, &simple.heredocs, cwd, &home)
            else {
                continue;
            };
            programs += 1;
            for (body, positioned) in fstrings(&source) {
                let rendered = pattern(&body);
                let w = worth(&rendered);
                total += 1;
                *by_worth.entry(w).or_insert(0) += 1;
                if positioned {
                    in_position += 1;
                    *positioned_worth.entry(w).or_insert(0) += 1;
                    *positioned_samples
                        .entry(w)
                        .or_default()
                        .entry(rendered.clone())
                        .or_insert(0) += 1;
                }
                *samples
                    .entry(w)
                    .or_default()
                    .entry(rendered.clone())
                    .or_insert(0) += 1;
                if w == Worth::Located
                    && let Some((dir, _)) = rendered.rsplit_once('/')
                {
                    *loci.entry(dir.to_string()).or_insert(0) += 1;
                }
            }
        }
    }

    println!("python programs read   {programs}");
    println!("interpolated f-strings {total}\n");
    for (w, n) in &by_worth {
        let pct = 100.0 * *n as f64 / total.max(1) as f64;
        println!("  {n:>6}  ({pct:>4.1}%)  {w:?}");
    }
    println!("\nof those, in a file operation's path argument: {in_position}");
    for (w, n) in &positioned_worth {
        let pct = 100.0 * *n as f64 / in_position.max(1) as f64;
        println!("  {n:>6}  ({pct:>4.1}%)  {w:?}");
    }
    for (w, seen) in &positioned_samples {
        println!("\nin path position, {w:?}:");
        let mut ranked: Vec<_> = seen.iter().collect();
        ranked.sort_by_key(|(text, n)| (std::cmp::Reverse(**n), (*text).clone()));
        for (text, n) in ranked.into_iter().take(show) {
            println!("  {n:>5}  {text}");
        }
    }
    for (w, seen) in &samples {
        println!("\n{w:?}, commonest first:");
        let mut ranked: Vec<_> = seen.iter().collect();
        ranked.sort_by_key(|(text, n)| (std::cmp::Reverse(**n), (*text).clone()));
        for (text, n) in ranked.into_iter().take(show) {
            println!("  {n:>5}  {text}");
        }
    }
    println!("\nloci a located f-string would name, commonest first:");
    let mut ranked: Vec<_> = loci.iter().collect();
    ranked.sort_by_key(|(dir, n)| (std::cmp::Reverse(**n), (*dir).clone()));
    for (dir, n) in ranked.into_iter().take(show) {
        println!("  {n:>5}  {dir}/");
    }
    Ok(())
}
