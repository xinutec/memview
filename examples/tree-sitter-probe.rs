//! Does a maintained grammar read this corpus better than ours?
//!
//!     cargo run --release --example tree-sitter-probe -- /tmp/bash-corpus.jsonl [--show <n>]
//!     cargo run --release --example tree-sitter-probe -- --parse-file <script>
//!
//! **Answered, 2026-08-06: no — it is a trade, not a gain, and the grammar stays.**
//! Over 98,321 distinct commands: ours reads 97,985 (99.7%), `tree-sitter-bash`
//! 0.25.1 reads 98,119 (99.8%). But the sets differ — **299 gained, 165 lost**,
//! a net 134 commands, 0.14%. And the losses are the shape that carries the most
//! meaning: `ssh host 'bash -s' <<'REMOTE'` and `nix develop --command python
//! <<'PY'`, which is where the remote work and the Python writes live.
//!
//! The loss has one cause, and it is upstream rather than ours to fix:
//!
//!     cat <<'EOF' 2>&1 | sed s/a/b/   → ERROR at the pipe
//!     cat <<'EOF' 2>&1 > /tmp/x       → clean
//!     cat <<'EOF' | sed s/a/b/        → clean
//!
//! A heredoc start, then another redirect, then a pipe. 128 of the 165.
//!
//! Kept rather than deleted so the decision can be re-taken against a later
//! release: run it again, and if the lost column empties the trade changes. An
//! example rather than a binary because that is what puts `tree-sitter` in
//! dev-dependencies, so nothing shipped carries a C toolchain for a measurement.
//!
//! Untested here and the likelier win: `tree-sitter-python` against
//! `python.pest`, which is the weaker of our two grammars — it cannot fail at
//! all, so the comparison there is about what is *understood*, not what parses.
//!
//! **A rate for each is not the answer; the cross-tabulation is.** Two parsers
//! reading 99% each could disagree about a different 1% and the swap would be a
//! trade rather than a gain. So every command is put through both and lands in
//! one of four cells, and the two disagreeing cells are printed with examples.
//!
//! ⚠ **tree-sitter cannot fail, and that is the thing to watch.** It is built for
//! editors, where a half-read file must still be highlighted: a construct it
//! cannot read becomes an ERROR or MISSING node and parsing carries on. Our
//! grammar refuses instead — an unclosed quote is an error, not a half-parse that
//! silently drops what follows — which is why our failures are a named list
//! rather than a quiet partial read, and half-reads are what invent paths. So a
//! tree is counted as read here ONLY when it holds no ERROR and no MISSING node
//! anywhere, which is the rule an adoption would have to keep.

use std::collections::BTreeMap;

use memview::shell;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: tree-sitter-probe <corpus.jsonl> [--show <n>]");
    };
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(8);

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&tree_sitter_bash::LANGUAGE.into())?;

    // One file, one tree, printed whole. The summary names the node an error sits
    // in; twice now on this project the surroundings were the actual problem.
    if let Some(i) = args.iter().position(|a| a == "--parse-file") {
        let text = std::fs::read_to_string(args.get(i + 1).expect("a path to parse"))?;
        let tree = parser.parse(&text, None).expect("bash always parses");
        println!("{}", tree.root_node().to_sexp());
        // And the text under each error, which the s-expression does not carry —
        // the node kind names the construct, the bytes name the actual problem.
        let mut stack = vec![tree.root_node()];
        while let Some(node) = stack.pop() {
            if node.is_error() || node.is_missing() {
                let kind = if node.is_missing() {
                    "MISSING"
                } else {
                    "ERROR"
                };
                println!(
                    "\n{kind} at {:?}: {:?}",
                    node.start_position(),
                    &text[node.byte_range()]
                );
            }
            let mut cursor = node.walk();
            stack.extend(node.children(&mut cursor).filter(|c| c.has_error()));
        }
        return Ok(());
    }

    // Whole failing commands, for reading — the same escape hatch `shell-report`
    // has, and needed here for the same reason.
    let mut dump = args
        .iter()
        .position(|a| a == "--dump-lost")
        .and_then(|i| args.get(i + 1))
        .map(std::fs::File::create)
        .transpose()?;

    let text = std::fs::read_to_string(path)?;
    let mut seen = std::collections::BTreeSet::new();
    // The four cells, and examples from the two that decide the question.
    let (mut both, mut ours_only, mut theirs_only, mut neither) = (0usize, 0usize, 0usize, 0usize);
    let mut gained: Vec<String> = Vec::new();
    let mut lost: Vec<String> = Vec::new();
    // What we would gain, grouped the way the shell report groups what we lack —
    // a worklist is more use than a number.
    let mut gained_by_shape: BTreeMap<&'static str, usize> = BTreeMap::new();
    // And what we would lose, by the node the error sits in.
    let mut lost_in: BTreeMap<String, usize> = BTreeMap::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        // Distinct commands, exactly as `shell-report` counts them, so the two
        // reports' denominators are the same number.
        if !seen.insert(cmd.to_string()) {
            continue;
        }

        let ours = shell::parse(cmd);
        let theirs = reads(&mut parser, cmd);
        match (ours.is_ok(), theirs) {
            (true, true) => both += 1,
            (false, true) => {
                theirs_only += 1;
                *gained_by_shape
                    .entry(shape_of(
                        ours.as_ref().err().map(String::as_str).unwrap_or(""),
                    ))
                    .or_default() += 1;
                if gained.len() < show {
                    gained.push(cmd.replace('\n', "⏎"));
                }
            }
            (true, false) => {
                ours_only += 1;
                if let Some(tree) = parser.parse(cmd, None) {
                    *lost_in.entry(where_error(tree.root_node())).or_default() += 1;
                }
                if lost.len() < show {
                    lost.push(cmd.replace('\n', "⏎"));
                }
                if let Some(dump) = &mut dump {
                    use std::io::Write;
                    writeln!(dump, "═════\n{cmd}")?;
                }
            }
            (false, false) => neither += 1,
        }
    }

    let total = both + ours_only + theirs_only + neither;
    let rate = |n: usize| 100.0 * n as f64 / total as f64;
    println!("{total} distinct commands");
    println!(
        "  ours   {:>6} ({:.1}%)",
        both + ours_only,
        rate(both + ours_only)
    );
    println!(
        "  theirs {:>6} ({:.1}%)   [no ERROR or MISSING node anywhere]",
        both + theirs_only,
        rate(both + theirs_only)
    );
    println!("both {both}  neither {neither}");
    println!("\ngained by the swap: {theirs_only} — ours fails, tree-sitter reads");
    for (shape, n) in {
        let mut v: Vec<_> = gained_by_shape.into_iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        v
    } {
        println!("  {n:>5}  {shape}");
    }
    for cmd in &gained {
        println!("    | {}", cut(cmd));
    }
    println!("\nLOST by the swap: {ours_only} — we read it, tree-sitter reports an error");
    for cmd in &lost {
        println!("    | {}", cut(cmd));
    }
    println!("\nwhere the loss sits, by the node containing the first error:");
    for (place, n) in {
        let mut v: Vec<_> = lost_in.into_iter().collect();
        v.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        v
    } {
        println!("  {n:>5}  {place}");
    }
    Ok(())
}

/// The kind of node the first error sits inside, walking down from the root.
///
/// A count of failures says how many; this says whether they are one thing. The
/// suspicion worth testing is heredocs — our reader treats a body as opaque text
/// and tree-sitter parses it, so a body that is Python or a remote script would
/// mark the whole tree bad for syntax that was never this shell's to read.
fn where_error(node: tree_sitter::Node) -> String {
    let mut at = node;
    let mut trail = vec![at.kind().to_string()];
    loop {
        let mut cursor = at.walk();
        let child = at
            .children(&mut cursor)
            .find(|c| c.has_error() || c.is_error() || c.is_missing());
        match child {
            Some(child) if child.child_count() > 0 && !child.is_error() => {
                trail.push(child.kind().to_string());
                at = child;
            }
            Some(child) => {
                trail.push(
                    if child.is_missing() {
                        format!("MISSING {}", child.kind())
                    } else {
                        child.kind().to_string()
                    }
                    .to_string(),
                );
                break;
            }
            None => break,
        }
    }
    // The last two are enough to name it and short enough to read in a column.
    let tail: Vec<String> = trail.into_iter().rev().take(2).collect();
    tail.into_iter().rev().collect::<Vec<_>>().join(" › ")
}

/// Whether the tree holds no error at all.
///
/// Both kinds, because they are different failures and either one means the tree
/// is not a reading of what was written: ERROR is text the grammar could not
/// place, MISSING is a node the grammar inserted to keep going — a closing quote
/// nobody wrote. `has_error` on the root covers both, and is checked on the root
/// because these propagate upward.
fn reads(parser: &mut tree_sitter::Parser, cmd: &str) -> bool {
    parser
        .parse(cmd, None)
        .is_some_and(|tree| !tree.root_node().has_error())
}

/// Where our parser stopped, named — the same grouping `shell-report` uses, so
/// the gained column can be read against the failure worklist directly.
fn shape_of(at: &str) -> &'static str {
    let head = at.trim_start();
    if head.starts_with("case") || head.starts_with(";;") {
        "case arm"
    } else if head.starts_with('(') || head.starts_with(')') {
        "subshell"
    } else if head.starts_with('{') || head.starts_with('}') {
        "brace group"
    } else if head.starts_with('\'') || head.starts_with('"') {
        "quoting"
    } else if head.starts_with('$') {
        "expansion"
    } else if head.is_empty() {
        "end of input"
    } else {
        "assorted"
    }
}

fn cut(cmd: &str) -> String {
    let mut out: String = cmd.chars().take(140).collect();
    if cmd.chars().count() > 140 {
        out.push('…');
    }
    out
}
