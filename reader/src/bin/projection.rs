//! Do the two readers agree about what ran?
//!
//!     cargo run --release -p reader --bin projection -- <corpus.jsonl> [--show <n>] [--only <bucket>]
//!
//! `reader/src/shell.rs` and `reader/src/syntax/` read the same text from two
//! grammars that share no code, and until now nothing compared them: the syntax
//! tree is gated against bash and against itself, the flat reader against a
//! handful of oracle fixtures, and **the two were never asked the same question.**
//! This asks it, over the whole corpus, by projecting the tree onto the flat
//! reader's own shape with [`reader::project`] and diffing the result.
//!
//! What comes out is the size of the port that would replace one with the other,
//! grouped so that each bucket is a decision rather than a list. A disagreement is
//! not automatically a defect on either side — the tree has structure the flat
//! reader never had, and the flat reader has words the tree refuses to invent —
//! so the buckets are named for the difference, and reading the samples is what
//! says which side was wrong.
//!
//! ⚠ **A bucket with a big count is not the biggest problem.** One systematic
//! spelling difference can outnumber every real misreading in the corpus, which
//! is why the samples are printed and why `--only` exists.

use std::collections::BTreeMap;

use reader::project::project;
use reader::shell::{self, Reached, Simple};
use reader::shell_ops::unwrap_command;
use reader::syntax;

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: projection <corpus.jsonl> [--show <n>] [--only <bucket>]");
    };
    let show: usize = flag(&args, "--show")
        .and_then(|n| n.parse().ok())
        .unwrap_or(3);
    let only = flag(&args, "--only");

    let text = std::fs::read_to_string(path)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut total = 0usize;
    let mut buckets: BTreeMap<&'static str, (usize, Vec<String>)> = BTreeMap::new();

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
        total += 1;
        let (bucket, detail) = verdict(cmd);
        let entry = buckets.entry(bucket).or_default();
        entry.0 += 1;
        if entry.1.len() < show && only.is_none_or(|name| name == bucket) {
            entry.1.push(format!("{}\n      {detail}", short(cmd)));
        }
    }

    println!("{total} distinct commands\n");
    let mut ranked: Vec<_> = buckets.iter().collect();
    ranked.sort_by_key(|(_, (count, _))| std::cmp::Reverse(*count));
    for (bucket, (count, samples)) in ranked {
        let share = 100.0 * *count as f64 / total as f64;
        println!("{count:>7}  {share:>5.2}%  {bucket}");
        for sample in samples {
            println!("      {sample}");
        }
    }
    Ok(())
}

fn flag<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .map(String::as_str)
}

/// One command's verdict: which bucket it falls in, and what to print for it.
fn verdict(cmd: &str) -> (&'static str, String) {
    let old = shell::parse(cmd);
    let new = syntax::parse(cmd).map(|script| project(&script));
    let (old, new) = match (old, new) {
        (Ok(old), Ok(new)) => (old, new),
        (Ok(_), Err(refusal)) => {
            return ("only the grammar reads it", format!("{:?}", refusal.reason));
        }
        (Err(why), Ok(_)) => return ("only the tree reads it", why),
        (Err(why), Err(refusal)) => {
            return ("neither reads it", format!("{:?} / {why}", refusal.reason));
        }
    };
    let old = compare(&old);
    let new = compare(&new);
    if old == new {
        return ("agree", String::new());
    }
    if old.len() != new.len() {
        return (
            "different commands",
            format!("grammar {:?}\n      tree    {:?}", heads(&old), heads(&new)),
        );
    }
    let Some((grammar, tree)) = old.iter().zip(&new).find(|(a, b)| a != b) else {
        unreachable!("unequal lists of equal length differ somewhere");
    };
    let bucket = if grammar.argv != tree.argv {
        words(&grammar.argv, &tree.argv)
    } else if grammar.reached != tree.reached {
        match (grammar.reached, tree.reached) {
            (Reached::Always, Reached::OnSuccess) => "condition: grammar certain, tree on-success",
            (Reached::Always, Reached::Sometimes) => "condition: grammar certain, tree uncertain",
            (Reached::OnSuccess, Reached::Always) => "condition: grammar on-success, tree certain",
            (Reached::Sometimes, Reached::Always) => "condition: grammar uncertain, tree certain",
            _ => "condition: other",
        }
    } else if grammar.depth != tree.depth {
        "different scope"
    } else if grammar.redirects != tree.redirects {
        "different redirections"
    } else if grammar.heredocs != tree.heredocs {
        "different heredocs"
    } else {
        unreachable!("keys differ in a field the buckets do not name")
    };
    (
        bucket,
        format!("grammar {grammar:?}\n      tree    {tree:?}"),
    )
}

/// Which *kind* of difference two argv lists have.
///
/// ⚠ **Because one systematic spelling difference outnumbers every real
/// misreading in the corpus.** The tree holds a word as typed segments and has to
/// spell an expansion back out to fill an argv string; the printer's spelling is
/// canonical, not the corpus's, so `${x}` comes back as `$x` and `$( a|b )` as
/// `$(a | b)`. Left in one bucket with the commands where a reader lost an
/// argument, those would bury them.
fn words(grammar: &[String], tree: &[String]) -> &'static str {
    let Some((left, right)) = grammar.iter().zip(tree).find(|(a, b)| a != b) else {
        return "words: one reader found more of them";
    };
    let without = |text: &str, unwanted: &str| -> String {
        text.chars().filter(|c| !unwanted.contains(*c)).collect()
    };
    for (unwanted, name) in [
        ("{}", "words: `${x}` against `$x`"),
        (" \t\n", "words: spacing inside an expansion"),
        ("\\", "words: an escape one reader resolved"),
        ("'\"", "words: quoting"),
    ] {
        if without(left, unwanted) == without(right, unwanted) {
            return name;
        }
    }
    "words: something else"
}

/// What a command is, for the purpose of asking whether two readers found the
/// same one.
#[derive(PartialEq, Eq, Debug)]
struct Key {
    argv: Vec<String>,
    reached: Reached,
    /// ⚠ **Depth, not the ids.** The two readers number their subshells in
    /// different orders — the tree walks a word's substitutions before the word
    /// and the grammar walks the pest children — so equal ids would be a
    /// coincidence and unequal ones say nothing. What must agree is how deeply
    /// nested the command is, because that is what decides whose `cd` it inherits.
    depth: usize,
    redirects: Vec<(String, bool)>,
    heredocs: Vec<String>,
}

/// The commands a reader found, with the grammar's own bookkeeping taken out.
///
/// ⚠ **This is the one asymmetry the comparison is not allowed to count.** The
/// flat grammar leaves `done`, `fi`, `esac` and the `for f in …` header behind as
/// ordinary commands, and three tables downstream exist to take them back out;
/// the tree has the structure they are a shadow of and emits none. Counting that
/// as a disagreement would put a five-figure number on the one difference both
/// sides already agree about.
fn compare(cmds: &[Simple]) -> Vec<Key> {
    const NOISE: [&str; 7] = ["done", "fi", "esac", "for", "case", "in", "select"];
    cmds.iter()
        .filter_map(|cmd| {
            let argv = unwrap_command(&cmd.argv);
            let head = argv.first()?;
            if NOISE.contains(&head.as_str()) {
                return None;
            }
            Some(Key {
                argv: argv.to_vec(),
                reached: cmd.reached,
                depth: cmd.scope.len(),
                redirects: cmd
                    .redirects
                    .iter()
                    .map(|r| (r.target.clone(), r.write))
                    .collect(),
                heredocs: cmd.heredocs.clone(),
            })
        })
        .collect()
}

/// The command names alone, for a sample line that has to fit on a screen.
fn heads(keys: &[Key]) -> Vec<&str> {
    keys.iter()
        .map(|key| key.argv.first().map_or("", String::as_str))
        .collect()
}

fn short(cmd: &str) -> String {
    let flat: String = cmd
        .chars()
        .map(|c| if c == '\n' { '⏎' } else { c })
        .collect();
    if flat.chars().count() > 110 {
        format!("{}…", flat.chars().take(110).collect::<String>())
    } else {
        flat
    }
}
