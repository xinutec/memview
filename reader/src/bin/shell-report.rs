//! How much of the history's shell the grammar can read, and what it cannot.
//!
//!     cargo run --bin shell-report -- <corpus.jsonl> [--show <n>]
//!
//! The corpus is one JSON object per line with a `cmd` field — every `Bash` call
//! in the transcripts. This is the loop the grammar is grown by: parse them all,
//! report the rate, and group the failures so the next construct to support is
//! the one at the top of the list rather than the one that comes to mind.
//!
//! It prints the failures' *shapes*, not just a count. A rate alone says how far
//! there is to go and nothing about where, and the whole point of starting from
//! a grammar too small is that its failures are a worklist.

use std::collections::BTreeMap;

use reader::shell;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: shell-report <corpus.jsonl> [--show <n>]");
    };
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(5);

    // Whole failing commands, for reading. The 24-character window in the
    // summary names the construct; it does not show what surrounds it, and twice
    // now the surroundings were the actual problem.
    let mut dump = args
        .iter()
        .position(|a| a == "--dump")
        .and_then(|i| args.get(i + 1))
        .map(std::fs::File::create)
        .transpose()?;

    let text = std::fs::read_to_string(path)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0usize;
    let mut ok = 0usize;
    let mut commands = 0usize;
    // Failures grouped by the construct that most likely caused them, so the
    // report names a feature to add rather than listing a thousand one-offs.
    let mut by_shape: BTreeMap<&'static str, (usize, Vec<String>)> = BTreeMap::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        // Distinct commands: the same one issued forty times is one thing to
        // support, and counting it forty times would flatter whatever it uses.
        if !seen.insert(cmd.to_string()) {
            continue;
        }
        total += 1;
        match shell::parse(cmd) {
            Ok(cmds) => {
                ok += 1;
                commands += cmds.len();
            }
            Err(at) => {
                let entry = by_shape.entry(shape_of(&at)).or_default();
                entry.0 += 1;
                if entry.1.len() < show {
                    entry.1.push(at.replace('\n', "⏎"));
                }
                if let Some(dump) = &mut dump {
                    use std::io::Write;
                    writeln!(dump, "───── stopped at: {}\n{cmd}", at.replace('\n', "⏎"))?;
                }
            }
        }
    }

    println!("distinct commands   {total}");
    println!(
        "parsed              {ok}  ({:.1}%)",
        100.0 * ok as f64 / total.max(1) as f64
    );
    println!("simple commands     {commands}");
    println!("\nfailures by shape (biggest first):");
    let mut shapes: Vec<_> = by_shape.into_iter().collect();
    shapes.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    for (shape, (n, samples)) in shapes {
        println!("  {n:>6}  {shape}");
        for s in samples {
            println!("            {}", truncate(&s, 96));
        }
    }
    Ok(())
}

/// Name the construct sitting at the position the parser stopped.
///
/// Keyed on what is actually *there*, not on what the whole command contains —
/// a command with a heredoc and a subshell fails at one of them, and guessing
/// from the whole text sent me after the wrong one twice.
fn shape_of(at: &str) -> &'static str {
    const PROBES: &[(&str, &str)] = &[
        ("(", "subshell / grouping ("),
        (")", "unbalanced )"),
        ("{", "brace group {"),
        ("}", "brace group }"),
        ("&", "background &"),
        ("<", "input redirection <"),
        (">", "redirection >"),
        ("`", "backtick"),
        ("$(", "command substitution"),
        ("!", "negation !"),
        (";", "stray ;"),
        ("|", "stray |"),
        ("do", "loop body"),
        ("done", "loop end"),
        ("then", "if body"),
        ("fi", "if end"),
        ("esac", "case end"),
        ("EOF", "unclosed heredoc"),
    ];
    for (needle, name) in PROBES {
        if at.starts_with(needle) {
            return name;
        }
    }
    if at.trim().is_empty() {
        return "end of input";
    }
    "other"
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    s.chars().take(n).collect::<String>() + "…"
}
