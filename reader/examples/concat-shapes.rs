//! What `base + name` still tells you about the path.
//!
//!     cargo run --release -p reader --example concat-shapes -- <corpus.jsonl> [--show <n>]
//!
//! memview#1142 landed slices for f-strings and for the three `join` spellings,
//! and left concatenation with an explicit instruction: **census it before
//! building anything.** The reason is a real asymmetry, not caution. A `/` join
//! KNOWS a separator goes between the parts, so `join(dir, 'meta.json')` renders
//! `*/meta.json`. `+` knows nothing of the sort: `base + name` renders `*` and
//! buys nothing at all unless a literal operand carries its own separator or
//! extension.
//!
//! So the question here is narrower than the f-string one was. Not "how much
//! literal text is there" but "how much of it is on the RIGHT SIDE of the join"
//! — a literal ending in `/` locates, a literal starting with `.` names, and a
//! literal in the middle of two variables does neither.
//!
//! ⚠ **This is lexical and approximate, deliberately, exactly as
//! `fstring-shapes` is.** It reads the operands immediately around a `+` and
//! renders a non-literal as `*`. It therefore misses `p = a + b` followed by
//! `open(p)`, which `scope()` handles, and it will read a `+` inside an
//! unrelated expression as a concatenation when a string literal happens to sit
//! beside it. Read the buckets against the positioned population, not the total.
//!
//! ⚠ **It does NOT call `python::path_shaped`.** Same argument the f-string
//! census makes: an instrument that asks the implementation's own question can
//! never show a shape the implementation gets wrong.
//!
//! **Answered 2026-09-01 over `bash-corpus.jsonl`.** 23,967 Python programs hold
//! 4,842 concatenations touching a literal, and over the whole population the
//! ticket's caution is right — 92.2% render `*` and buy nothing. Filtered to a
//! file operation's path argument the count is 245 and the distribution
//! inverts:
//!
//!     29.8%  Nothing    `*`, `rs*`, a print line that happens to sit in one
//!     63.7%  Language   `*.md`, `*/*.json`, `*/evidence.jsonl` — the NAME is certain
//!      6.5%  Located    `eval/scan3d/*`, `/tmp/negmem/*`
//!
//! So **172 of 245 would gain a real answer**, and the balance matches the JOIN
//! slice rather than the f-strings: mostly filenames, rarely a directory. Do not
//! size a locus-only rule from this.
//!
//! ⚠ **The first run of this census reported 1.** `operand` returned the index
//! where its scan STOPPED rather than where the operand BEGAN, so
//! `in_path_position` asked whether `open(ba` ends with `open(`. A control
//! corpus of four hand-written programs — three concatenations sitting literally
//! inside `open(`/`Path(` — scored zero and exposed it. An instrument that
//! reports a small number is indistinguishable from one that cannot fire, so
//! prove it fires before believing a negative result.

use std::collections::BTreeMap;

use reader::shell_ops::{self, Op};

/// What a rendered concatenation is worth as a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Worth {
    /// `*`, `**` — every file there is. The commonest outcome for `+`.
    Nothing,
    /// `*.json`, `*.stream` — no directory is certain, but the NAME is.
    Language,
    /// `logs/*`, `data/*.json` — the text before the first hole holds a
    /// separator, so a directory is certain too.
    Located,
}

/// The file operations whose path argument is the population this is about.
///
/// Kept identical to `fstring-shapes`'s list on purpose: the two censuses size
/// slices of the same rule, and a different denominator would make them
/// incomparable.
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

/// Whether the expression starting at `at` sits in a file operation's argument.
fn in_path_position(source: &[char], at: usize) -> bool {
    let from = at.saturating_sub(24);
    let before: String = source[from..at].iter().collect();
    OPENERS.iter().any(|opener| before.ends_with(opener))
}

/// Bucket a rendered pattern, on the ONE question that matters for `+`: does a
/// literal operand carry a separator or an extension?
fn worth(pattern: &str) -> Worth {
    let literal: String = pattern.chars().filter(|c| *c != '*').collect();
    if literal.trim().is_empty() || literal.contains([' ', '\t']) || pattern.contains("://") {
        return Worth::Nothing;
    }
    // A separator or a trailing extension is the whole of what `+` can buy.
    let extension = pattern
        .rsplit('*')
        .next()
        .is_some_and(|tail| tail.starts_with('.') && tail.len() > 1 && !tail.contains('/'));
    if !literal.contains('/') && !extension {
        return Worth::Nothing;
    }
    // `*/*` passes everything above and names neither a directory nor a file.
    if literal.chars().all(|c| c == '/' || c == '.') {
        return Worth::Nothing;
    }
    match pattern.split('*').next().unwrap_or_default().contains('/') {
        true => Worth::Located,
        false => Worth::Language,
    }
}

/// One operand of a `+`, rendered: a string literal becomes its text, anything
/// else becomes `*`.
///
/// Returns the operand's text and the index where it BEGINS in source order —
/// not where the scan stopped. ⚠ Returning the scan end instead is what made the
/// first run of this census report 1 positioned concatenation in the whole
/// corpus: `in_path_position` looks at the text ENDING at the index it is given,
/// so a mid-token index asked whether `open(ba` ends with `open(`, which nothing
/// ever does. The control corpus caught it — three concatenations sitting
/// literally inside `open(`/`Path(` scored zero.
fn operand(source: &[char], mut i: usize, forward: bool) -> Option<(String, usize)> {
    // Skip the whitespace either side of the operator.
    loop {
        let c = *source.get(i)?;
        if !c.is_whitespace() {
            break;
        }
        i = if forward { i + 1 } else { i.checked_sub(1)? };
    }
    let c = *source.get(i)?;
    if c == '\'' || c == '"' {
        // A literal. Scan it whole, in whichever direction we are walking.
        let mut body = String::new();
        let mut j = i;
        loop {
            j = if forward { j + 1 } else { j.checked_sub(1)? };
            let d = *source.get(j)?;
            if d == c {
                break;
            }
            if d == '\n' {
                return None;
            }
            body.push(d);
        }
        if !forward {
            body = body.chars().rev().collect();
        }
        // Walking left, `j` is the opening quote — the operand's start. Walking
        // right, the operand started at the quote we entered on.
        return Some((body, if forward { i } else { j }));
    }
    // Anything else — a name, a call, a subscript — is unknown. Walk to the
    // start of the token so the caller can ask what precedes the whole operand.
    if !forward {
        let mut begin = i;
        while begin > 0 {
            let c = source[begin - 1];
            if c.is_alphanumeric() || c == '_' || c == '.' || c == ')' || c == ']' {
                begin -= 1;
            } else {
                break;
            }
        }
        return Some(("*".to_string(), begin));
    }
    Some(("*".to_string(), i))
}

/// Every `a + b` in a program's text whose operands touch a string literal,
/// rendered with `*` for the unknown parts and NO separator inserted.
fn concats(source: &str) -> Vec<(String, bool)> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    for i in 0..chars.len() {
        if chars[i] != '+' || chars.get(i + 1) == Some(&'=') {
            continue;
        }
        if i > 0 && (chars[i - 1] == '+' || chars[i - 1] == '=') {
            continue;
        }
        let Some((left, left_at)) = operand(&chars, i.wrapping_sub(1), false) else {
            continue;
        };
        let Some((right, _)) = operand(&chars, i + 1, true) else {
            continue;
        };
        // At least one side has to be literal, or there is nothing to say.
        if left == "*" && right == "*" {
            continue;
        }
        let mut rendered = format!("{left}{right}");
        // Adjacent unknowns are one run, as in the f-string census.
        while rendered.contains("**") {
            rendered = rendered.replace("**", "*");
        }
        found.push((rendered, in_path_position(&chars, left_at)));
    }
    found
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: concat-shapes <corpus.jsonl> [--show <n>]");
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
            for (rendered, positioned) in concats(&source) {
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
            }
        }
    }

    println!("python programs read   {programs}");
    println!("concatenations touching a literal {total}\n");
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
        println!("\n{w:?} — most common, in a path argument:");
        let mut rows: Vec<_> = seen.iter().collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (pattern, n) in rows.into_iter().take(show) {
            println!("  {n:>4}  {pattern}");
        }
    }
    Ok(())
}
