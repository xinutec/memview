//! What the semantics table can read out of the history's shell, and what it
//! cannot.
//!
//!     cargo run --release --bin shell-files -- <corpus.jsonl> [--show <n>] [--paths <n>]
//!
//! The companion to `shell-report`, which measures the *grammar*. This measures
//! the layer above it: of the commands that parse, how many does the table in
//! `shell_files.rs` understand, and which unread commands are the biggest.
//!
//! **Counted per call, not per distinct command** — the opposite of
//! `shell-report`, and deliberately. There, forty runs of one command are one
//! construct to support and counting them forty times would flatter it. Here
//! frequency is the whole signal: a command run four thousand times is worth
//! adding to the table and one run once is not.

use std::collections::BTreeMap;

use anyhow::Context;

use reader::shell_files;
use reader::shell_ops::{GitOp, Op};

/// The shape of an operation, for the distribution.
fn op_name(op: &Op) -> &'static str {
    match op {
        Op::Read { .. } => "read",
        Op::Write { .. } => "write",
        Op::Remove { .. } => "remove",
        Op::Copy { .. } => "copy",
        Op::Move { .. } => "move",
        Op::Search { .. } => "search",
        Op::Transform { in_place: true, .. } => "transform (in place)",
        Op::Transform { .. } => "transform",
        Op::Run { .. } => "run a script",
        Op::Nested { .. } => "open a shell (bash -c, nix --run)",
        Op::Python { .. } => "run python (-c, or a heredoc)",
        Op::Remote { .. } => "reach another machine (ssh, kubectl exec)",
        Op::ChangeDir { .. } => "cd",
        Op::Git(GitOp::Stage { .. }) => "git stage",
        Op::Git(GitOp::Alter { .. }) => "git alter",
        Op::Git(GitOp::Inspect { .. }) => "git inspect",
        Op::Git(GitOp::Other { .. }) => "git (other)",
        Op::Nothing => "nothing with files",
        Op::Unknown { .. } => "not understood",
    }
}

/// Shorten for display, on character boundaries.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.replace('\n', "⏎");
    }
    s.chars().take(n).collect::<String>().replace('\n', "⏎") + "…"
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let Some(path) = args.get(1) else {
        anyhow::bail!("usage: shell-files <corpus.jsonl> [--show <n>] [--paths <n>]");
    };
    let count = |flag: &str, default: usize| -> usize {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .and_then(|n| n.parse().ok())
            .unwrap_or(default)
    };
    let show = count("--show", 25);
    let paths = count("--paths", 0);
    // The check that matters: not how many paths came out, but whether a given
    // one came from a command that really names it. Every doubt about this table
    // has been settled by reading the commands behind a single suspicious path.
    let why = args
        .iter()
        .position(|a| a == "--why")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;
    let mut calls = 0usize;
    let mut unparsed = 0usize;
    let mut handled = 0usize;
    let mut unhandled = 0usize;
    let mut by_name: BTreeMap<String, usize> = BTreeMap::new();
    // Subjects the text does not determine. Counted beside the commands that were
    // not read at all, because they are the same kind of admission: the size of
    // what this does not know, stated by the thing that does not know it.
    let mut unnamed = 0usize;
    let mut by_word: BTreeMap<String, usize> = BTreeMap::new();
    // Subjects a glob loop BOUNDED — still unnamed, but a subset of a pattern
    // rather than of anything at all.
    let mut by_pattern: BTreeMap<String, usize> = BTreeMap::new();
    // The same admission from the Python reader: the call that wanted a path,
    // where the path itself was computed and no text of it survives.
    let mut computed: BTreeMap<String, usize> = BTreeMap::new();
    // And the uses Python DID name that this layer's own rules turned away.
    let mut turned_away = reader::python::Refused::default();
    let mut reads = 0usize;
    let mut writes = 0usize;
    // Distinct paths, so the size of what this produces is visible rather than
    // implied by a total that double-counts every file opened twice.
    let mut distinct: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut witnessed = 0usize;
    // Which commands actually open and change files — the check anyone runs
    // before trusting the table, and the one that showed `sed` to be a pager.
    let mut by_command: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    // What the typed model knows that a path and a direction cannot: the shape
    // of the work, what was being looked for, and what got renamed.
    let mut by_op: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut searched: BTreeMap<String, usize> = BTreeMap::new();
    let mut renames = 0usize;
    let mut nested_unparsed = 0usize;
    let mut unrolled = 0usize;
    // File uses by what had to hold for the command naming them to run.
    let (mut always, mut on_success, mut sometimes) = (0usize, 0usize, 0usize);
    // Those the call's outcome confirms actually happened.
    let mut certain = 0usize;
    // What happens on the other machines — read from the same scripts, kept out
    // of every local figure.
    let mut remote: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut remote_paths: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();

    for line in text.lines() {
        let Ok(row) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(cmd) = row["cmd"].as_str() else {
            continue;
        };
        let cwd = row["cwd"].as_str().filter(|c| !c.is_empty());
        // The `cd` targets the shell refused, which only its own output knows —
        // see `agents::refusals`. Absent on all but a handful of rows.
        let refused: Vec<String> = row["refused"]
            .as_array()
            .map(|it| {
                it.iter()
                    .filter_map(|t| t.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        // What became of the call. A corpus written before outcomes were
        // recorded has no such field, and that silence is `Unknown` rather than
        // success — the figures below then show only what runs unconditionally,
        // which is the honest answer to "with no outcome, what is certain".
        //
        // An outcome that is *present and unreadable* is a different thing
        // entirely, and is an error: quietly reading it as `Unknown` would turn
        // a corrupt corpus into a modest-looking one.
        let ran: reader::doing::Verdict = match row.get("ran") {
            None | Some(serde_json::Value::Null) => reader::doing::Verdict::Unknown,
            Some(outcome) => serde_json::from_value(outcome.clone())
                .with_context(|| format!("unreadable outcome in the corpus: {outcome}"))?,
        };
        calls += 1;
        // ⚠ **The same reader the chain uses, and there is no flag for the
        // other one.** `shell_files` reads a nested `bash -c` through the tree
        // whichever reader opened the outer script, so a second column here
        // would measure a mixture rather than a reader. Both readers are
        // compared a layer earlier, by `--bin projection`, where neither has to
        // be switched to do it.
        let Ok(parsed) = reader::project::read(cmd) else {
            unparsed += 1;
            continue;
        };
        let found = shell_files::extract_knowing(&parsed, cwd, &home, &refused);
        handled += found.handled;
        nested_unparsed += found.nested_unparsed;
        unrolled += found.unrolled;
        for use_ in &found.remote {
            let host = remote.entry(use_.host.clone()).or_default();
            let path = remote_paths
                .entry((use_.host.clone(), use_.path.clone()))
                .or_default();
            if use_.write {
                host.1 += 1;
                path.1 += 1;
            } else {
                host.0 += 1;
                path.0 += 1;
            }
        }
        for (name, (r, w)) in &found.by_command {
            let entry = by_command.entry(name.clone()).or_default();
            entry.0 += r;
            entry.1 += w;
        }
        for op in &found.ops {
            *by_op.entry(op_name(op)).or_insert(0) += 1;
            match op {
                Op::Search { pattern, .. } if !pattern.is_empty() => {
                    *searched.entry(pattern.clone()).or_insert(0) += 1;
                }
                Op::Move { .. } => renames += 1,
                _ => {}
            }
        }
        for (name, n) in &found.unhandled {
            unhandled += n;
            *by_name.entry(name.clone()).or_insert(0) += n;
        }
        // ⚠ The total comes from `subjects_not_named`, which folds in the Python
        // reader's own account too — the map below is the shell's words alone,
        // and reading a total off it is the undercount memview#824 was about.
        unnamed += found.subjects_not_named();
        for (word, n) in &found.unnamed {
            *by_word.entry(word.clone()).or_insert(0) += n;
        }
        for (pattern, n) in &found.bounded {
            *by_pattern.entry(pattern.clone()).or_insert(0) += n;
        }
        for (call, n) in &found.python.unresolved {
            *computed.entry(call.clone()).or_insert(0) += n;
        }
        turned_away.merge(&found.python.refused);
        for file in found.files {
            if let Some(why) = &why
                && file.path.contains(why.as_str())
                && witnessed < show
            {
                witnessed += 1;
                let mark = if file.write { "write" } else { "read " };
                println!("{mark}  {}\n       {}\n", file.path, cmd.replace('\n', "⏎"));
            }
            match file.reached {
                reader::shell::Reached::Always => always += 1,
                reader::shell::Reached::OnSuccess => on_success += 1,
                reader::shell::Reached::Sometimes => sometimes += 1,
            }
            if ran.admits(file.reached) {
                certain += 1;
            }
            let entry = distinct.entry(file.path).or_default();
            if file.write {
                writes += 1;
                entry.1 += 1;
            } else {
                reads += 1;
                entry.0 += 1;
            }
        }
    }

    let commands = handled + unhandled;
    println!("Bash calls          {calls}");
    println!("  unparsed          {unparsed}");
    // Commands *run*, not commands written: a determinate loop is run out into
    // its iterations before any of this counts them. Stated next to the total
    // because it moves the denominator under every percentage below.
    println!("simple commands     {commands}");
    println!("  from unrolling    {unrolled}");
    println!(
        "  understood        {handled}  ({:.1}%)",
        100.0 * handled as f64 / commands.max(1) as f64
    );
    println!("  not in the table  {unhandled}");
    // A wrapper whose inner shell will not parse is a hole in exactly the third
    // of the corpus that runs through one, so it is counted rather than shrugged
    // at — the same rule as every other refusal here.
    println!("  nested, unparsed  {nested_unparsed}");
    println!("file uses           {} reads, {writes} writes", reads);
    println!("  ran regardless    {always}   on `&&` {on_success}   conditional {sometimes}");
    println!(
        "  certainly ran     {certain}  ({} unconfirmable)",
        always + on_success + sometimes - certain
    );
    println!("distinct paths      {}", distinct.len());
    // ⚠ **Stated as a rate against the uses, not left as a bare count.** These are
    // subjects a command named and this reader could not: without them the line
    // above reads as "every file that was used", which is the overstatement the
    // count exists to end.
    // ⚠ **One denominator, covering both readers.** Split out because the three
    // are different admissions: a shell word the text does not determine, a
    // Python path the program computed, and a use this layer's own rules turned
    // away. Only the first two are unknowable; the third is a rule that could be
    // revisited, and a single total hid which was which.
    println!(
        "subjects not named  {unnamed}  ({:.1}% of all uses)",
        100.0 * unnamed as f64 / (reads + writes + unnamed).max(1) as f64
    );
    println!(
        "  shell, by word    {}  ({} distinct)",
        by_word.values().sum::<usize>(),
        by_word.len()
    );
    // ⚠ Bounded, not named: a subset of a pattern is not a file. Shown apart
    // because the difference between "some subset of `src/*.ts`" and "some file"
    // is the whole of what a constrained unknown buys.
    println!(
        "  shell, bounded    {}  ({} distinct patterns)",
        by_pattern.values().sum::<usize>(),
        by_pattern.len()
    );
    println!(
        "  python, computed  {}  ({} distinct calls)",
        computed.values().sum::<usize>(),
        computed.len()
    );
    println!(
        "  refused here      {}   moved {}   no directory {}   not a path {}",
        turned_away.total(),
        turned_away.moved,
        turned_away.no_directory,
        turned_away.not_a_path
    );

    println!("\nwhat the shell was doing:");
    let mut shapes: Vec<_> = by_op.into_iter().collect();
    shapes.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), *name));
    for (name, n) in shapes {
        println!("  {n:>7}  {name}");
    }
    println!("  {renames:>7}  of those were renames, which no read/write count can express");

    println!("\nmost searched-for terms:");
    let mut terms: Vec<_> = searched.into_iter().collect();
    terms.sort_by_key(|(term, n)| (std::cmp::Reverse(*n), term.clone()));
    for (term, n) in terms.into_iter().take(show) {
        println!("  {n:>7}  {}", truncate(&term, 68));
    }

    println!("\ncommands that CHANGE files, biggest first:");
    let mut writers: Vec<_> = by_command
        .iter()
        .filter(|(_, (_, w))| *w > 0)
        .map(|(name, (r, w))| (name.clone(), *r, *w))
        .collect();
    writers.sort_by_key(|(name, _, w)| (std::cmp::Reverse(*w), name.clone()));
    for (name, reads, writes) in writers.into_iter().take(show) {
        println!("  {writes:>7} writes  {reads:>7} reads   {name}");
    }

    println!("\nfiles used on OTHER machines (never counted as local work):");
    let mut hosts: Vec<_> = remote.into_iter().collect();
    hosts.sort_by_key(|(host, (r, w))| (std::cmp::Reverse(r + w), host.clone()));
    for (host, (r, w)) in hosts.into_iter().take(show) {
        println!("  {r:>6} read {w:>5} written   {host}");
    }
    let mut busiest: Vec<_> = remote_paths.into_iter().collect();
    busiest.sort_by_key(|((host, path), (r, w))| {
        (std::cmp::Reverse(r + w), host.clone(), path.clone())
    });
    for ((host, path), (r, w)) in busiest.into_iter().take(show) {
        println!("      {r:>5}r {w:>4}w  {host}:{path}");
    }

    println!("\nsubjects the text does not determine, biggest first:");
    let mut words: Vec<_> = by_word.into_iter().collect();
    words.sort_by_key(|(word, n)| (std::cmp::Reverse(*n), word.clone()));
    for (word, n) in words.iter().take(show.max(10)) {
        println!("  {n:>7}  {}", truncate(word, 68));
    }

    println!("\nunread commands, biggest first:");
    let mut ranked: Vec<_> = by_name.into_iter().collect();
    ranked.sort_by_key(|(name, n)| (std::cmp::Reverse(*n), name.clone()));
    for (name, n) in ranked.into_iter().take(show) {
        println!("  {n:>7}  {name}");
    }

    if paths > 0 {
        println!("\nbusiest paths (reads/writes):");
        let mut ranked: Vec<_> = distinct.into_iter().collect();
        ranked.sort_by_key(|(path, (r, w))| (std::cmp::Reverse(r + w), path.clone()));
        for (path, (r, w)) in ranked.into_iter().take(paths) {
            println!("  {r:>6} {w:>6}  {path}");
        }
    }
    Ok(())
}
