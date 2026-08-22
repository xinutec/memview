//! What the fleet's commands carry that nothing looks inside.
//!
//!     cargo run --release --bin opacity -- <corpus.jsonl> [--show <n>] [--why|--dump <label>]
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
//! ⚠ **Asked again from the other side, 2026-08-17, and it came out the same.**
//! Once the sniff stopped calling TypeScript "Python", TypeScript became the
//! biggest carried language by bytes — 698 bodies, 953 kB — which reads like a
//! reason to build one. `--dump` hands the bodies out and the tally settles it:
//! **19 of the 698 write a file, 23 writes in total**, 241 only read, and 450
//! touch no file at all. The same 23 the command-side count found. A body is
//! large because it embeds a TypeScript AST walk, not because it does much, so
//! **size ranked this wrongly and the operations ranked it right**. memview#1034.
//!
//! ⚠ **The language of a heredoc body is SNIFFED, and a sniff is a guess.** It
//! is reported as "looks like" for that reason. The point is the ranking — is
//! there a kilobyte of SQL here or a megabyte — not the label on any one body.
//!
//! ⚠ **And the ranking was wrong for as long as the guess had no tests.** On
//! 2026-08-17 the Python bucket stood at 1,154 bodies and 2.0 MB; **731 of them
//! were not Python.** One line opening `import ` decided the language, and
//! TypeScript, Kotlin, Swift and Lean all open a line that way — as does an
//! English sentence that wraps onto a line beginning "from". Corrected, the
//! biggest carried language is **TypeScript or JavaScript at 698 bodies and
//! 953 kB**, ahead of Python's 423 and 850 kB, and Lean and Swift have buckets
//! at all. `--why <label>` prints a bucket's bodies with the mark that filed
//! each one, which is what makes the next such error findable rather than
//! plausible.
//!
//! ⚠ **The 4.4 MB of "prose" is mostly not a gap.** `cat`, `git` and `task`
//! opened 5,209 of those bodies — a heredoc redirected into a file, a commit
//! message, a task body. The first is a write to a path already known and the
//! other two are not programs at all, so the megabytes there rank far below
//! what their size suggests. Read this census by *opener* as well as by
//! language.

use std::collections::BTreeMap;

use reader::shell_files;
use reader::shell_ops::Op;
use reader::sniff::looks_like;

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
        anyhow::bail!("usage: opacity <corpus.jsonl> [--show <n>] [--why|--dump <label>]");
    };
    let show: usize = args
        .iter()
        .position(|a| a == "--show")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse().ok())
        .unwrap_or(12);
    // Which sniffed label to open up, rather than count.
    let why: Option<String> = args
        .iter()
        .position(|a| a == "--why")
        .and_then(|i| args.get(i + 1))
        .map(|label| label.to_lowercase());
    // ⚠ **A bucket's size never says whether reading it is worth it.** That
    // takes looking at what the bodies *do*, and no reader exists for the
    // language in question — which is the very thing being decided. So the
    // bodies go out NUL-separated for whatever can answer, the same bargain
    // `nested-why` and `python-raised` strike with `bash -n` and `ast.parse`.
    let dump: Option<String> = args
        .iter()
        .position(|a| a == "--dump")
        .and_then(|i| args.get(i + 1))
        .map(|label| label.to_lowercase());
    let home = std::env::var("HOME").unwrap_or_default();

    let text = std::fs::read_to_string(path)?;
    let mut calls = 0usize;
    let mut unread: BTreeMap<String, usize> = BTreeMap::new();
    // The payloads, by what the command meant to do with them.
    let mut kinds: BTreeMap<&'static str, Weighed> = BTreeMap::new();
    // Heredoc bodies by the language they look like.
    let mut bodies: BTreeMap<&'static str, Weighed> = BTreeMap::new();
    // Which command opened each unread body.
    let mut openers: BTreeMap<String, usize> = BTreeMap::new();
    // Words we refuse to resolve, by why.
    let mut unresolved: BTreeMap<&'static str, Weighed> = BTreeMap::new();
    // Under `--why`: the bodies filed under that label, each with the mark that
    // put it there, tallied so one shape repeated 400 times reads as one line.
    let mut opened: BTreeMap<(&'static str, String), usize> = BTreeMap::new();

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
        calls += 1;
        let Ok(parsed) = reader::project::read(cmd) else {
            continue;
        };

        // Unresolvable words come off the parse: they are a property of the text
        // as written rather than of what it does.
        for simple in &parsed.commands {
            for word in &simple.argv {
                if let Some(why) = refuses(word) {
                    unresolved.entry(why).or_default().add(word, show);
                }
            }
        }

        let found = shell_files::extract_knowing(&parsed, cwd, &home, &refused);
        // ⚠ **Only the bodies nobody read.** The first run of this counted every
        // heredoc, so 6.1 MB of Python that `python.rs` reads in full was filed
        // under opacity — and a census that counts work already done ranks the
        // next reader wrongly. A body handed to a reader is not dark.
        let carried: Vec<&str> = found
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Python { source } | Op::JavaScript { source } => Some(source.as_str()),
                Op::Nested { script } => Some(script.as_str()),
                Op::Remote { script, .. } => Some(script.as_str()),
                // Its words went to a reader too — as an argv, which carries no
                // heredoc, so there is no body of its own to exempt here.
                Op::RemoteRun { .. } => None,
                _ => None,
            })
            .collect();
        for simple in &parsed.commands {
            for body in &simple.heredocs {
                if carried.iter().any(|source| source.contains(body.as_str())) {
                    continue;
                }
                let (guess, mark) = looks_like(body);
                bodies.entry(guess).or_default().add(body, show);
                if why
                    .as_deref()
                    .is_some_and(|label| guess.to_lowercase().contains(label) || label == "all")
                {
                    *opened.entry((mark, cut(body))).or_default() += 1;
                }
                if dump
                    .as_deref()
                    .is_some_and(|label| guess.to_lowercase().contains(label) || label == "all")
                {
                    print!("{body}\0");
                }
                // ⚠ **And by the command that opened it, which is the question
                // that actually decides anything.** A body fed to an interpreter
                // is a program we cannot read; a body redirected into a file is
                // that file's new contents, and the *effect* — a write, to a path
                // we already know — is captured whether or not anyone reads the
                // text. Ranking bodies by language conflates the two.
                let opener = simple.argv.first().map_or("(a redirect)", |word| {
                    word.rsplit('/').next().unwrap_or(word)
                });
                *openers.entry(opener.to_string()).or_default() += 1;
            }
        }
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

    // ⚠ `--why` answers a different question and prints instead of, not beside,
    // the census: the point is to read what a bucket actually holds, and a
    // thousand bodies below a summary is not something anybody reads.
    if dump.is_some() {
        return Ok(());
    }
    if let Some(label) = &why {
        println!("bodies sniffed as {label:?}, by the mark that decided it\n");
        let mut sorted: Vec<(&(&str, String), &usize)> = opened.iter().collect();
        sorted.sort_by_key(|((mark, _), n)| (*mark, std::cmp::Reverse(**n)));
        for ((mark, body), n) in sorted {
            println!("  {n:>5}  [{mark}]  {body}");
        }
        return Ok(());
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
        // ⚠ **The samples are the point for the bucket that has no name.** A
        // count of 3,589 unrecognised bodies is not a worklist; the first line of
        // a dozen of them is. Printed for every bucket rather than only that one,
        // because a sniff is a guess and the way to check a guess is to read what
        // it guessed about.
        for one in weighed.seen.iter().take(6) {
            println!("           | {one}");
        }
    }

    println!("\n── who opened the bodies nobody reads ──");
    ranked(&openers, show);

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
