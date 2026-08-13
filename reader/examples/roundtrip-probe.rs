//! Does re-rendering a parse and reading it again give the same parse back?
//!
//! A probe, not a check — kept beside `tree-sitter-probe.rs` and for the same
//! reason: it answers a design question with a number, and the number is worth
//! more than the argument it settles.
//!
//! **The question.** A round-trip property (render the tree, parse it again,
//! expect the same tree) is the classic cheap way to buy enormous test coverage:
//! every one of the corpus's 115,578 distinct commands becomes a case for free.
//! Before building it, it is worth knowing whether it can *fail* — a property
//! that holds by construction tests the renderer and nothing else.
//!
//! ⚠ **The suspicion going in was that it is vacuous, and it was wrong.** The
//! reasoning: `argv` holds words with quotes already stripped, so a correct
//! renderer single-quotes every word, and a single-quoted word parses back to
//! itself by definition. Plausible, and false — measured 2026-08-13 over 105,594
//! commands, **88.50% identical and 12,074 not**, all of them one bug:
//!
//!     word as parsed:  === does 'life' appear at all? ===
//!     rendered back:   '=== does '\''life'\'' appear at all? ==='
//!     read again:      === does \life\ appear at all? ===
//!
//! `'\''` is how POSIX puts a quote inside a quoted string. The parser keeps the
//! backslash and drops the quote — memview#833, and 274 corpus commands hit it
//! without any help from this probe.
//!
//! **One cause, not twelve thousand**: checked directly, 12,070 of the differing
//! commands contain a single quote and 0 do not. Which is how a property behaves
//! — one rule, met everywhere. Once #833 is fixed this should read near 100%, and
//! that figure is the ratchet worth keeping.
//!
//! **What is compared, and what cannot be.** `Reached` and `scope` have no
//! surface syntax to render back to: `&&` is gone into a three-point domain and a
//! subshell into an id. Rendering with newlines makes every command `Always` at
//! the top level, so those two fields are excluded and counted separately. What
//! is compared is `argv` and `redirects` — the parts a renderer can actually put
//! back on the page.
//!
//!     cargo run --release --example roundtrip-probe -- <corpus.jsonl>

use std::collections::BTreeMap;

use reader::shell::{Simple, parse};

/// One word, spelled so that reading it again yields exactly this word.
///
/// Single quotes, because inside them the shell expands nothing at all — which
/// is the whole point: an `argv` element is a *value*, and any spelling that
/// could expand would be a different claim about it.
fn quoted(word: &str) -> String {
    if word.is_empty() {
        return "''".to_string();
    }
    // The one character single quotes cannot hold: close, escape it, reopen.
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// One command, spelled canonically. `None` when it cannot be spelled at all.
fn render(cmd: &Simple) -> Option<String> {
    // ⚠ A heredoc body cannot be put back without inventing a delimiter that the
    // body does not contain, and choosing one is a renderer's problem rather than
    // the parser's. Counted apart rather than guessed at.
    if !cmd.heredocs.is_empty() {
        return None;
    }
    let mut words: Vec<String> = cmd.argv.iter().map(|word| quoted(word)).collect();
    for redirect in &cmd.redirects {
        words.push(if redirect.write {
            ">".into()
        } else {
            "<".into()
        });
        words.push(quoted(&redirect.target));
    }
    (!words.is_empty()).then(|| words.join(" "))
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: roundtrip-probe <corpus.jsonl>"))?;
    let text = std::fs::read_to_string(&path)?;

    let mut seen = std::collections::BTreeSet::new();
    let (mut distinct, mut unparsed, mut heredocs, mut empty) = (0usize, 0usize, 0usize, 0usize);
    let (mut same, mut differ, mut broke) = (0usize, 0usize, 0usize);
    // What the differences look like, so a non-zero answer names a cause rather
    // than a count.
    let mut shapes: BTreeMap<&'static str, (usize, Vec<String>)> = BTreeMap::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        if !seen.insert(cmd.to_string()) {
            continue;
        }
        distinct += 1;
        let Ok(first) = parse(cmd) else {
            unparsed += 1;
            continue;
        };
        let mut rendered = Vec::new();
        let mut skipped = false;
        for command in &first {
            match render(command) {
                Some(line) => rendered.push(line),
                None => {
                    skipped = true;
                    break;
                }
            }
        }
        if skipped {
            heredocs += 1;
            continue;
        }
        if rendered.is_empty() {
            empty += 1;
            continue;
        }
        let script = rendered.join("\n");
        let Ok(again) = parse(&script) else {
            // The renderer produced something this parser cannot read. That is a
            // real finding whichever side is at fault.
            broke += 1;
            let entry = shapes.entry("re-parse FAILED").or_default();
            entry.0 += 1;
            if entry.1.len() < 3 {
                entry.1.push(script.chars().take(90).collect());
            }
            continue;
        };
        // ⚠ `reached` and `scope` are excluded on purpose — see the module note.
        let alike = first.len() == again.len()
            && first.iter().zip(&again).all(|(a, b)| {
                a.argv == b.argv
                    && a.redirects.len() == b.redirects.len()
                    && a.redirects
                        .iter()
                        .zip(&b.redirects)
                        .all(|(x, y)| x.target == y.target && x.write == y.write)
            });
        if alike {
            same += 1;
        } else {
            differ += 1;
            let why = if first.len() != again.len() {
                "a different NUMBER of commands"
            } else if first
                .iter()
                .zip(&again)
                .any(|(a, b)| a.argv.len() != b.argv.len())
            {
                "a different number of WORDS"
            } else {
                "the same shape, different words"
            };
            let entry = shapes.entry(why).or_default();
            entry.0 += 1;
            if entry.1.len() < 3 {
                entry.1.push(cmd.chars().take(90).collect());
            }
        }
    }

    println!("distinct commands   {distinct}");
    println!("  unparsed          {unparsed}   (nothing to render)");
    println!("  heredoc, skipped  {heredocs}   (a delimiter cannot be invented safely)");
    println!("  rendered to empty {empty}");
    let tried = same + differ + broke;
    println!("round-tripped       {tried}");
    println!(
        "  identical         {same}  ({:.2}%)",
        100.0 * same as f64 / tried.max(1) as f64
    );
    println!("  DIFFERENT         {differ}");
    println!("  re-parse failed   {broke}");
    if !shapes.is_empty() {
        println!("\nwhere it did not hold:");
        for (why, (n, examples)) in &shapes {
            println!("  {n:>6}  {why}");
            for example in examples {
                println!("          {}", example.replace('\n', "⏎"));
            }
        }
    }
    Ok(())
}
