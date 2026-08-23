//! What the semantics table can read out of the history's shell, and what it
//! cannot.
//!
//!     cargo run --release --bin shell-files -- <corpus.jsonl> [--show <n>] [--paths <n>]
//!
//! The companion to `shell-report`, which measures the *grammar*. This measures
//! the layer above it: of the commands that parse, how many does the table in
//! `shell_files.rs` understand, and which unread commands are the biggest.
//!
//! ⚠ **This binary computes nothing.** The survey is [`reader::reading`], and
//! this is one view of it — the API serving the same numbers to the console is
//! another. When it was the only consumer it held the accumulation itself, and
//! the second consumer is exactly the moment that stops being free: two
//! calculations of "how much is understood" drift apart silently, and nothing in
//! either would say so.

use reader::reading::Reading;

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
    let why = args
        .iter()
        .position(|a| a == "--why")
        .and_then(|i| args.get(i + 1))
        .cloned();
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;
    // ⚠ **The same reader the chain uses, and there is no flag for the other
    // one.** `shell_files` reads a nested `bash -c` through the tree whichever
    // reader opened the outer script, so a second column here would measure a
    // mixture rather than a reader. Both readers are compared a layer earlier,
    // by `--bin projection`, where neither has to be switched to do it.
    let read = match &why {
        Some(why) => Reading::of_corpus_watching(&text, &home, why, show)?,
        None => Reading::of_corpus(&text, &home)?,
    };

    for (write, path, cmd) in &read.witnesses {
        let mark = if *write { "write" } else { "read " };
        println!("{mark}  {path}\n       {}\n", cmd.replace('\n', "⏎"));
    }

    println!("Bash calls          {}", read.calls);
    println!("  unparsed          {}", read.unparsed);
    // Commands *run*, not commands written: a determinate loop is run out into
    // its iterations before any of this counts them. Stated next to the total
    // because it moves the denominator under every percentage below.
    println!("simple commands     {}", read.commands());
    println!("  from unrolling    {}", read.unrolled);
    println!(
        "  understood        {}  ({:.1}%)",
        read.handled,
        read.understood()
    );
    println!("  not in the table  {}", read.unhandled);
    // A wrapper whose inner shell will not parse is a hole in exactly the third
    // of the corpus that runs through one, so it is counted rather than shrugged
    // at — the same rule as every other refusal here.
    // ⚠ **Ranked, not totalled.** A nested script that will not read is a whole
    // script's worth of file uses lost, and a bare number names no construct to
    // build — which is how this sat at 405 for a day saying nothing (#1028).
    println!(
        "  nested, unparsed  {}",
        read.nested_unparsed.values().sum::<usize>()
    );
    let mut ranked: Vec<_> = read.nested_unparsed.iter().collect();
    ranked.sort_by_key(|(reason, n)| (std::cmp::Reverse(**n), (*reason).clone()));
    for (reason, n) in ranked.iter().take(8) {
        println!("      {n:>5}  {reason}");
    }
    println!(
        "file uses           {} reads, {} writes",
        read.reads, read.writes
    );
    println!(
        "  ran regardless    {}   on `&&` {}   conditional {}",
        read.always, read.on_success, read.sometimes
    );
    println!(
        "  certainly ran     {}  ({} unconfirmable)",
        read.certain,
        read.always + read.on_success + read.sometimes - read.certain
    );
    println!("distinct paths      {}", read.distinct.len());
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
        "subjects not named  {}  ({:.1}% of all uses)",
        read.unnamed,
        read.opaque()
    );
    println!(
        "  shell, by word    {}  ({} distinct)",
        read.by_word.values().sum::<usize>(),
        read.by_word.len()
    );
    // ⚠ Bounded, not named: a subset of a pattern is not a file. Shown apart
    // because the difference between "some subset of `src/*.ts`" and "some file"
    // is the whole of what a constrained unknown buys.
    println!(
        "  shell, bounded    {}  ({} distinct patterns)",
        read.by_pattern.values().sum::<usize>(),
        read.by_pattern.len()
    );
    println!(
        "  python, computed  {}  ({} distinct calls)",
        read.computed.values().sum::<usize>(),
        read.computed.len()
    );
    println!(
        "  refused here      {}   moved {}   no directory {}   not a path {}",
        read.turned_away.total(),
        read.turned_away.moved,
        read.turned_away.no_directory,
        read.turned_away.not_a_path
    );

    println!("\nwhat the shell was doing:");
    let mut shapes: Vec<_> = read.by_op.iter().collect();
    shapes.sort_by_key(|(name, n)| (std::cmp::Reverse(**n), **name));
    for (name, n) in shapes {
        println!("  {n:>7}  {name}");
    }
    println!(
        "  {:>7}  of those were renames, which no read/write count can express",
        read.renames
    );

    println!("\nmost searched-for terms:");
    let mut terms: Vec<_> = read.searched.iter().collect();
    terms.sort_by_key(|(term, n)| (std::cmp::Reverse(**n), (*term).clone()));
    for (term, n) in terms.into_iter().take(show) {
        println!("  {n:>7}  {}", truncate(term, 68));
    }

    println!("\ncommands that CHANGE files, biggest first:");
    let mut writers: Vec<_> = read
        .by_command
        .iter()
        .filter(|(_, (_, w))| *w > 0)
        .map(|(name, (r, w))| (name.clone(), *r, *w))
        .collect();
    writers.sort_by_key(|(name, _, w)| (std::cmp::Reverse(*w), name.clone()));
    for (name, reads, writes) in writers.into_iter().take(show) {
        println!("  {writes:>7} writes  {reads:>7} reads   {name}");
    }

    println!("\nfiles used on OTHER machines (never counted as local work):");
    let mut hosts: Vec<_> = read.remote.iter().collect();
    hosts.sort_by_key(|(host, (r, w))| (std::cmp::Reverse(r + w), (*host).clone()));
    for (host, (r, w)) in hosts.into_iter().take(show) {
        println!("  {r:>6} read {w:>5} written   {host}");
    }
    let mut busiest: Vec<_> = read.remote_paths.iter().collect();
    busiest.sort_by_key(|((host, path), (r, w))| {
        (std::cmp::Reverse(r + w), host.clone(), path.clone())
    });
    for ((host, path), (r, w)) in busiest.into_iter().take(show) {
        println!("      {r:>5}r {w:>4}w  {host}:{path}");
    }

    println!("\nsubjects the text does not determine, biggest first:");
    let mut words: Vec<_> = read.by_word.iter().collect();
    words.sort_by_key(|(word, n)| (std::cmp::Reverse(**n), (*word).clone()));
    for (word, n) in words.iter().take(show.max(10)) {
        println!("  {n:>7}  {}", truncate(word, 68));
    }

    println!("\nunread commands, biggest first:");
    let mut ranked: Vec<_> = read.by_name.iter().collect();
    ranked.sort_by_key(|(name, n)| (std::cmp::Reverse(**n), (*name).clone()));
    for (name, n) in ranked.into_iter().take(show) {
        println!("  {n:>7}  {name}");
    }

    if paths > 0 {
        println!("\nbusiest paths (reads/writes):");
        let mut ranked: Vec<_> = read.distinct.iter().collect();
        ranked.sort_by_key(|(path, (r, w))| (std::cmp::Reverse(r + w), (*path).clone()));
        for (path, (r, w)) in ranked.into_iter().take(paths) {
            println!("  {r:>6} {w:>6}  {path}");
        }
    }
    Ok(())
}
