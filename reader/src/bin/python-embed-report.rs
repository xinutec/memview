//! The Python the tree finds inside each command, beside what the flat chain
//! finds, over the history's commands. The two are separate answers to one
//! question, so where they differ one of them is wrong or a layer is missing.
//!
//!     cargo run --release -p reader --bin python-embed-report -- <corpus.jsonl> [--show <kind> <n>]
//!
//! `<kind>` is `tree-only` or `flat-only`.

use std::collections::BTreeSet;

use reader::shell_ops::Op;
use reader::syntax::embed::{Program, python};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: python-embed-report <corpus.jsonl> [--show <kind> <n>]");
    };
    let show = args.iter().position(|a| a == "--show").and_then(|at| {
        Some((
            args.get(at + 1)?.clone(),
            args.get(at + 2)?.parse::<usize>().ok()?,
        ))
    });
    let home = std::env::var("HOME").unwrap_or_default();

    let (mut commands, mut sites, mut read, mut refused, mut expands) = (0, 0, 0, 0, 0);
    let (mut both, mut tree_only, mut flat_only, mut flat_expands) = (0, 0, 0, 0);
    let mut shown = 0;
    let mut seen = BTreeSet::new();
    for line in std::fs::read_to_string(path)?.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        if !seen.insert(cmd.to_string()) {
            continue;
        }
        let Ok(script) = reader::syntax::parse(cmd) else {
            continue;
        };
        commands += 1;

        let mut tree = BTreeSet::new();
        let mut tree_expands = 0;
        for embedded in python(&script) {
            sites += 1;
            match embedded.program {
                Program::Text { source, tree: t } => {
                    if t.is_ok() {
                        read += 1;
                    } else {
                        refused += 1;
                    }
                    tree.insert(source);
                }
                Program::Expands { .. } => {
                    expands += 1;
                    tree_expands += 1;
                }
            }
        }
        let mut flat = BTreeSet::new();
        if let Ok(parsed) = reader::project::read(cmd) {
            let found =
                reader::shell_files::extract_knowing(&parsed, row["cwd"].as_str(), &home, &[]);
            for op in found.ops {
                if let Op::Python { source } = op {
                    flat.insert(source);
                }
            }
        }

        both += tree.intersection(&flat).count();
        let only_tree: Vec<&String> = tree.difference(&flat).collect();
        let only_flat: Vec<&String> = flat.difference(&tree).collect();
        // A site the tree calls expanding still reached the flat chain as text with
        // the `$` left in; those are one program read two ways, not a gap.
        let explained = only_flat.len().min(tree_expands);
        flat_expands += explained;
        tree_only += only_tree.len();
        flat_only += only_flat.len() - explained;

        if let Some((kind, n)) = &show
            && shown < *n
        {
            let hit = match kind.as_str() {
                "tree-only" => !only_tree.is_empty(),
                "flat-only" => only_flat.len() > explained,
                _ => false,
            };
            if hit {
                shown += 1;
                println!("--- {kind}:\n{cmd}");
                let differing = if kind == "tree-only" {
                    &only_tree
                } else {
                    &only_flat
                };
                for program in differing {
                    println!("  program:\n{program}");
                }
                println!();
            }
        }
    }

    println!("distinct commands the tree reads  {commands}");
    println!("python sites found                {sites}");
    println!("  read by the python tree         {read}");
    println!("  refused by the python tree      {refused}");
    println!("  text expands in the shell       {expands}");
    println!("programs, tree against flat chain");
    println!("  found by both                   {both}");
    println!("  tree only                       {tree_only}");
    println!("  flat only                       {flat_only}");
    println!("  flat, where the tree expands    {flat_expands}");
    Ok(())
}
