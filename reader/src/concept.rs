//! What a command was *for*, as a thing that can be turned back into a command.
//!
//! The first lens of `docs/concept-model.md`. [`crate::shell_ops`] says what a
//! command did to files and [`crate::activity`] names the kind of work; neither
//! can be run backwards, and [`crate::activity`] says so about itself. This is
//! the level that carries enough to regenerate — a **representation** rather
//! than a classification — and the difference is the whole point of it.
//!
//! ## The law
//!
//! ```text
//! lift ∘ lower = id        lowering a concept and lifting it back is identity
//! lower ∘ lift ≠ id        permitted: the text normalises — layout, quoting,
//!                          and here the SPELLING (`sed -i` for `perl -pi`)
//! ```
//!
//! The same shape as the syntax layer's round-trip law, one level up, and with
//! the same constraint carried with it: **the concept is sufficient**. [`lower`]
//! takes a [`Concept`] and nothing else — no `Step`, no source text — because a
//! concept that can only be printed by consulting the command it came from is an
//! annotation, not a concept.
//!
//! ## Why `Rewrite` first, and why it is measured rather than chosen
//!
//! `sed -i 's/a/b/' f` and `perl -pi -e 's/a/b/' f` reach the **identical**
//! [`Op::Transform { program, in_place }`], so two spellings meet in one concept
//! and the cross-language claim has something real to assert. `Page` was the
//! intuitive first pick and is the wrong one: [`Op::Read`] keeps only paths, so
//! `head -5 f` and `cat f` are one key and the range is gone (memview#1364).
//!
//! ⚠ **Shell-only, and that is a correction to the design doc.** The merge one
//! level down is thinner than it claimed: the type both carried readers share is
//! `FileUse { path, write, reached }`, so `python::record` never extracts
//! `re.sub`'s pattern and a Python `Rewrite` would have nothing to compare
//! against. Cross-language needs a parameter field on `program.rs` first.
//!
//! ## Where it attaches
//!
//! [`crate::shell_files::Step`], which is the only place a command and its
//! reading are both in hand. Not "a pure L2 reading", which is what the doc said
//! and what the projection refutes: `operands()` drops flags by construction, so
//! no seed concept's parameters survive at the `Op` alone.

use crate::shell_files::Step;
use crate::shell_ops::{Op, basename, unwrap_command};

/// A concept's parameter, carrying the precision the reader had and no more.
///
/// ⚠ **The three-part artefact, at this level.** The reader's whole discipline
/// is that a lower bound, a described middle and a counted remainder are
/// different claims; a concept that flattened them to `Option<String>` would
/// throw away the half that is falsifiable. So a subject the text located keeps
/// its locus, a glob keeps its language, and a value that was never in the text
/// stays a [`Subject::Hole`] rather than becoming a guess or a `⊤`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Subject {
    /// A path the text determined.
    Named(String),
    /// An unknown member of a known language — `S ⊆ L`, the glob's own pattern.
    Bounded(String),
    /// A directory the answer is rooted at, with the leaf unknown.
    Located(String),
    /// A value that was never in the text: stdin, a parameter, a name bound
    /// outside the program. `program::Why::Outside`, one level up.
    Hole,
}

/// Which part of a file a [`Concept::Page`] shows.
///
/// ⚠ **The range is the point, and the projection below throws it away** — the
/// census measured that (memview#1364): `Op::Read` keeps only paths, so
/// `head -5 f` and `cat f` are one key there. A `Page` that dropped the range
/// too would be a second name for `Read`. So the range is read off
/// [`Step::argv`], the one place it survives.
///
/// The vocabulary is closed to the shapes the corpus actually spells (measured
/// 2026-09-04): a count from the top (`head`), a count from the bottom
/// (`tail`), an explicit line span (`sed -n 'a,bp'`), or the whole file
/// (`cat`). A byte count, a follow (`tail -f`), a `+N` prefix drop, and a
/// `$`-relative address are **different acts** and refuse rather than flatten
/// to one of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Range {
    /// The whole file — `cat`.
    All,
    /// The first `n` lines — `head -n`, and `sed -n '1,np'`, which measure to
    /// the same thing.
    First(u32),
    /// The last `n` lines — `tail -n`.
    Last(u32),
    /// An explicit line span `a..=b`, `a > 1` — `sed -n 'a,bp'`. A single line
    /// `sed -n 'np'` is `Lines(n, n)`.
    Lines(u32, u32),
}

/// The language a [`Concept::Search`] pattern is written in.
///
/// ⚠ **This is MEANING, not spelling, so it is carried rather than normalised
/// away.** `a|b` matches the three characters under basic grep and either letter
/// under `-E` — measured, both. A concept that dropped the dialect would lower
/// to a command matching different lines, which is the one thing [`lower`] may
/// not do. Contrast `sed -i` standing in for `perl -pi`, where the language is
/// genuinely spelling because the act is identical.
///
/// The three the corpus spells. `grep -P` and rg's own dialect are neither of
/// these and refuse rather than flatten to [`Pattern::Extended`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pattern {
    /// A basic regular expression — `grep`'s default, where `|` and `+` are
    /// ordinary characters.
    Basic(String),
    /// An extended one — `grep -E`, `egrep`.
    Extended(String),
    /// A fixed string, no metacharacters at all — `grep -F`, `fgrep`.
    Fixed(String),
}

/// Which number a [`Concept::Measure`] hands back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantity {
    /// `wc -l`.
    Lines,
    /// `wc -w`.
    Words,
    /// `wc -c` — bytes, for all the letter says character; the character count
    /// is [`Quantity::Chars`], which POSIX spells `-m`.
    Bytes,
    /// `wc -m`.
    Chars,
}

/// What a command was for.
///
/// The vocabulary is mined and admitted the way a syntax construct was — biggest
/// first (the census ranks it), refused by name until built — and a catch-all
/// `Run { argv }` would take the lift rate to 100% on the first day and mean
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Concept {
    /// A file changed in place by a program applied to its contents.
    ///
    /// `subjects` is a list because the command's own shape is: `sed -i 's/a/b/'
    /// a.ts b.ts` is one act over two files, and splitting it into two concepts
    /// would lower to two commands, which is a different program.
    Rewrite {
        subjects: Vec<Subject>,
        /// The substitution as written — `s/a/b/`. A [`Subject::Hole`]'s
        /// equivalent here is `None`: `sed -i -f fix.sed x` rewrites by a
        /// program that is in another file and not in this text.
        substitution: Option<String>,
    },
    /// A file (or none, for a stream) shown without being changed — the corpus's
    /// largest concept-shaped mass by a distance (census 2026-09-04: the `head`,
    /// `tail`, `cat` and `sed -n` pager shapes together dwarf every `Rewrite`).
    ///
    /// ⚠ **Its spellings do NOT meet at one `Op`, and that recast gate 2.**
    /// `head -5 f` is [`Op::Read`] and `sed -n '1,5p' f` is [`Op::Transform`]
    /// printing — the reader below reads two operations for one act. The concept
    /// is where they meet, so the level-below authority a lowered `Page` answers
    /// to is the L3 *effect* reading (what was touched, in which direction), not
    /// the `Op` variant. `reader/tests/concept.rs::read_as` carries the reason.
    Page {
        /// Empty for a stream: `… | head -50` pages what flows in, and no file
        /// was named. Not a hole — a hole is a subject the text gestured at and
        /// could not resolve; this is a subject that was never there.
        subjects: Vec<Subject>,
        range: Range,
    },
    /// The lines of a file that match a pattern — the largest shape left in the
    /// queue after `Page` (census 2026-09-10: 119,690 rows across every search
    /// spelling, of which this lens accepts 65,209).
    ///
    /// ⚠ **Only the act that PRODUCES MATCHING LINES.** `-c` counts them, `-l`
    /// names the files, `-q` answers yes or no and prints nothing, `-o` prints
    /// the matched fragment rather than the line. Each is a different product
    /// from the same scan, so each refuses by name and the census sizes it —
    /// 22,630 rows between them, which is what a later lens would be worth.
    Search {
        /// Empty for a stream: `… | grep -n foo` searches what flows in. Not a
        /// hole, for the reason [`Concept::Page`] gives.
        subjects: Vec<Subject>,
        pattern: Pattern,
        /// `-i`. Carried rather than dropped because it changes which lines come
        /// back, exactly as the dialect does.
        fold_case: bool,
        /// `-r` — the subjects are trees to walk, not files to read. Load-bearing
        /// for the lowering: `grep pattern dir/` without it is an error, so a
        /// concept that dropped it would lower to a command that fails.
        descend: bool,
    },
    /// The entries of a directory — `ls`.
    ///
    /// ⚠ **The product is NAMES, and that is the whole boundary.** `ls -l` adds
    /// size, mode and time; `du` adds bytes; `wc -l` returns a count. Each reads
    /// the same locus and hands back something else, so each refuses by name.
    /// Measured 2026-09-10: of 19,376 `ls` rows, 11,378 are a bare `ls <dir>`
    /// and ~5,800 carry the `-l` family.
    ///
    /// ⚠ **`find` is NOT this concept, and the census is why.** Its operands are
    /// a predicate EXPRESSION — 1,277 rows use `-o`, 1,242 `-not`, 281 `-prune`
    /// — so no single `matching` field represents it, and keeping only the
    /// `-name` value would claim a NARROWER set than the command walked. That is
    /// a false lower bound, which is the direction this reader refuses
    /// everywhere else. [`Why::Predicate`] holds it, counted.
    List {
        /// The directories enumerated. `ls a b` is one act over two loci, the
        /// same way `sed -i` is over two files.
        loci: Vec<Subject>,
        /// `-R`. The same field `Search` carries, for the same reason: it
        /// changes which names come back.
        descend: bool,
        /// `-a`. Also changes which names come back, so it is carried rather
        /// than normalised away — `ls d` and `ls -a d` are different sets.
        hidden: bool,
    },
    /// One number about a file's CONTENTS — `wc`.
    ///
    /// ⚠ **The product is a COUNT, which is the same boundary drawn again.**
    /// The locus `cat` shows and `grep` scans, read for a number instead:
    /// `grep -c` refuses ([`Why::NotLines`] — a count of MATCHES is a
    /// different question), and `du`/`stat` are numbers about the FILE rather
    /// than its contents — metadata, the [`Concept::List`] `-l` boundary — and
    /// stay queued with the census sizing them (2026-09-05: `wc -l` 7,979
    /// rows, `wc -c` 1,365, `du -sh` 724, the `stat` shapes ~480 and carrying
    /// format strings besides).
    ///
    /// ⚠ **One quantity.** Bare `wc` is the POSIX lines-words-bytes triple — a
    /// real default, readable not writable — but a TABLE is a different
    /// product from a number, and neither it nor a combined flag made the
    /// census. Queued, not modelled.
    Measure {
        /// Empty for a stream: `… | wc -l` counts what flows in. Not a hole,
        /// for the reason [`Concept::Page`] gives.
        subjects: Vec<Subject>,
        quantity: Quantity,
    },
    /// The most recent commits of a repository — `git log`.
    ///
    /// ⚠ **The FIRST concept whose subject is not a file.** A repository is
    /// context, not an operand: `git log -3` names nothing, and the `-C` that
    /// could name one is a location rather than a subject. So there is no repo
    /// field — the concept says what the text says, and where it ran is the
    /// step's business, exactly as a relative path's directory is.
    ///
    /// ⚠ **`git log`'s dominant shape is one shape.** Measured 2026-09-10 over
    /// 23,160 steps: 94% carry `--oneline` and 87% a count, and 20,846 (90%)
    /// need nothing but a count, a revision and paths after `--`.
    History {
        /// `-3`, `-n 3`, `--max-count=3`. `None` where the text gave none, which
        /// is git's own unbounded default — NOT a number invented here, and the
        /// difference matters because `git log` and `git log -1` are very
        /// different amounts of output.
        count: Option<u32>,
        /// `git log 5710b66`, `git log HEAD~4..HEAD` — where history is read
        /// FROM. Carried as written, the way [`Concept::Rewrite`] carries a
        /// substitution: it is a literal in the text, opaque to this reader, and
        /// faithful when written back.
        from: Option<String>,
        /// Paths after a `--`, which the author has DECLARED to be paths — the
        /// same guarantee [`crate::shell_ops::GitOp::Inspect`] relies on. Empty
        /// is the ordinary case and means the whole repository.
        paths: Vec<Subject>,
    },
    /// What a repository has that its last commit does not — `git status`.
    ///
    /// ⚠ **Essentially parameterless, and the census is why.** Of 15,436 steps,
    /// 88 carry no flag at all and almost every flag that appears is FORMAT:
    /// `--short` (9,089), `--porcelain` (4,779), `-sb` (1,391), `-b`. They
    /// choose a spelling of the same listing, so they normalise away exactly as
    /// `--oneline` does for [`Concept::History`].
    Status {
        /// Restricted to these paths where the text gave any. Empty means the
        /// whole tree, which is what almost every occurrence means.
        paths: Vec<Subject>,
    },
    /// Files put into the index — `git add`.
    ///
    /// ⚠ **Staging changes NO file**, which the level below already says: it is
    /// [`crate::shell_ops::GitOp::Stage`], a variant that exists precisely to
    /// keep that decision visible. The concept inherits it — an ask card must
    /// not read a stage as a write.
    Stage {
        subjects: Vec<Subject>,
        /// `-A`. Load-bearing rather than decoration: it stages deletions and
        /// everything else in the tree, so the set is not the operands. With no
        /// operand it means the whole repository, which the lift refuses for the
        /// reason [`Why::ImplicitLocus`] gives.
        all: bool,
    },
    /// A commit written — `git commit`.
    ///
    /// ⚠ **The message is the AUTHOR'S OWN description of the work**, which is
    /// the thing gate 4 wants and has never had in the argv. It is carried as
    /// written.
    Commit {
        /// `None` where the message is real and NOT in this text — `-F file`,
        /// or `-F -` reading stdin, which is 2,972 of 10,758 steps. Exactly the
        /// sense [`Concept::Rewrite`]'s `substitution` is `None`.
        message: Option<String>,
        amend: bool,
        /// `--no-verify` — the hooks do not run. Carried because it is the one
        /// thing an approver most needs told: this commit skips the gate.
        no_verify: bool,
    },
}

/// Why a step did not lift.
///
/// ⚠ **The census's key, born with the layer.** #1142 rebuilt three temporary
/// inventories before keying misses by reason, and the method learned from that
/// (`docs/concept-model.md`): the layer starts with its `Why`, so the remainder
/// is never a bare count. [`Why::NoLens`] is the queue — ranked by shape, it is
/// where the next concept comes from. The rest are the lenses' own refusals:
/// steps a lens *looked at* and turned down, each a design question the census
/// sizes ([`Why::Described`] is "does a concept need to lower to a loop",
/// counted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// No lens covers this shape. The counted remainder, and the queue.
    NoLens,
    /// A carrier — `bash -c`, `ssh`, `kubectl exec`. Its content arrives as
    /// steps of its own and is lifted there; a concept on the carrier too
    /// would say the child's work twice, the same double-count `Step.files`
    /// refuses for a wrapper's paths.
    Carrier,
    /// A transform that prints rather than edits — a different act, and the
    /// pool a `Page` lens would draw from.
    NotInPlace,
    /// Ran on another machine; a lowered local command would claim the wrong
    /// world.
    Remote,
    /// A described subject — `S ⊆ L`, a loop's language — which no single
    /// command can lower without promoting it to a false lower bound.
    Described,
    /// A search whose product is not the matching lines — `-c` counts, `-l` and
    /// `-L` name files, `-q` answers with an exit status, `-o` prints fragments.
    /// The same scan, a different answer.
    NotLines,
    /// A search that shows more than the matches — `-A`, `-B`, `-C` — or fewer,
    /// `-m` stopping at a cap. The span is the point, the way a `Page`'s range
    /// is, so flattening it would claim lines the command did not show.
    WithContext,
    /// `grep -v` — the complement of the pattern. Every line BUT the matches is
    /// a different set, and a lens that ignored the flag would name the exact
    /// lines the command suppressed.
    Inverted,
    /// `--include`, `--exclude-dir`, rg's `--type` — a filter on which files are
    /// searched, which [`Subject`] has no way to say. A lowered form without it
    /// would search more than the command did.
    Filtered,
    /// `grep -e PAT` and `grep -f FILE` put the pattern somewhere other than the
    /// leading operand, and [`crate::shell_ops::Op::Search`] takes the leading
    /// operand as the pattern — so lifting these would name a FILE as the thing
    /// searched for.
    PatternInFlag,
    /// The text named an operand the level below could not turn into a subject.
    ///
    /// ⚠ **Refused because SILENCE HERE READS AS A STREAM.** `grep -rn foo src`
    /// loses `src` — [`crate::shell_ops::looks_like_path`] cannot tell a bare
    /// word from a bare directory, and says so — and it is not admitted as a
    /// hole either, because most such words are genuinely not files. So the
    /// subjects come back empty, which is the same shape a piped `… | grep foo`
    /// produces, and the concept would claim nothing was named when something
    /// was. Counting the operands is what tells the two apart.
    UnreadSubject,
    /// A listing that hands back more than names — `ls -l` and its family add
    /// mode, size and time. Same locus, different product, which is the same
    /// call [`Why::NotLines`] makes for `grep -c`.
    WithMetadata,
    /// `ls -d` names the DIRECTORY rather than enumerating it — the one flag
    /// that inverts the act instead of adjusting it.
    NotTheContents,
    /// `find`'s operands are a predicate EXPRESSION — `-o`, `-not`, `-prune`,
    /// `-type`, `-newermt` — not a pattern. Flattening it to the `-name` value
    /// would claim a narrower walk than the command made, so it is refused whole
    /// and counted until a concept exists that can hold a predicate.
    Predicate,
    /// A listing with no operand: `ls` alone enumerates the working directory.
    /// The locus is real and IMPLICIT, so the concept would have to write a
    /// subject the text never did. Distinct from a stream, where no subject
    /// exists at all — which is why it does not reuse [`Why::UnreadSubject`].
    ImplicitLocus,
    /// A `git log` whose commits are not "the most recent N" — `--all` reads
    /// every ref, `--since` a time window, `--grep` and `-S` search the history
    /// itself, `--diff-filter` and `--follow` filter by what changed. Each picks
    /// a different SET, and a concept that dropped the flag would name commits
    /// the command never showed.
    OtherSelection,
    /// A `git log` whose product is not a list of commits — `--format` selects
    /// fields, `-p` prints patches, `--stat` a diffstat, `--name-only` filenames.
    /// The same commits, a different answer, which is the call [`Why::NotLines`]
    /// makes for `grep -c`.
    Formatted,
}

/// Lift one step into the concept it served, or say why not.
///
/// ⚠ **The refusal is the honest answer and must stay cheap to give.** A command
/// with no concept stays an L2/L3 leaf and is counted; that is what keeps a lift
/// rate from being manufactured, and it is the same rule the parser follows when
/// it refuses a construct by name. A caller that wants only the concept takes
/// `.ok()`; the census reads the other arm, and the two cannot drift because
/// there is one function.
pub fn lift(step: &Step) -> Result<Concept, Why> {
    // ⚠ **A carrier is refused before its op is read as work** — its children
    // are steps of their own and are lifted there. See [`Why::Carrier`].
    if matches!(
        &step.op,
        Some(Op::Nested { .. } | Op::Remote { .. } | Op::RemoteRun { .. })
    ) {
        return Err(Why::Carrier);
    }
    match &step.op {
        Some(Op::Transform {
            program,
            program_file,
            paths,
            in_place: true,
        }) => {
            let subjects = subjects_or_refuse(step, paths)?;
            Ok(Concept::Rewrite {
                subjects,
                // A program given as a file is a hole in the same sense a path
                // is: the substitution exists, and not in this text.
                substitution: program_file.is_none().then(|| program.clone()),
            })
        }
        // ⚠ **A NOT-in-place transform is a `sed -n` page, or it prints and is
        // a different act.** `sed 's/a/b/' f` writes the whole file back to
        // stdout with the edit; a lowered `Page` would claim it showed a span,
        // which it did not. Only a bare line-address program under `-n` pages.
        Some(Op::Transform { program, paths, .. }) => match sed_page(step, program) {
            Some((range, operands)) => page(step, paths, range, operands),
            None => Err(Why::NotInPlace),
        },
        // ⚠ **A read is a page only for the four pagers, and only in the
        // shapes the corpus spells.** `wc -l` and `od` also reach [`Op::Read`];
        // they MEASURE rather than show, and stay counted leaves. `ls` and
        // `find` reach it too and are the [`Concept::List`] act — which is why
        // the page reader is asked first and the listing reader second, rather
        // than either of them owning the variant.
        Some(Op::Read { paths }) => match read_page(step) {
            Some((range, operands)) => page(step, paths, range, operands),
            // Only a shape no earlier reader RECOGNISED falls through — a named
            // refusal (`find`, `ls -l`) is an answer and must not be re-asked.
            None => match listing(step, paths) {
                Err(Why::NoLens) => measure(step, paths),
                other => other,
            },
        },
        Some(Op::Search { pattern, paths }) => {
            let shape = search_shape(step)?;
            // Every flag this lens accepts is valueless, so the pattern is the
            // first operand and each word after it is a subject. `None` when
            // there are no operands at all — `grep -- pat` counts none, because
            // [`search_shape`] stops counting at `--` — and it must refuse at
            // the COUNT check, not before it: measured 2026-09-10, nine remote
            // `grep -- pat` steps refuse as Remote, which outranks a miscount
            // in every lens.
            let subjects = counted_subjects(step, paths, shape.operands.checked_sub(1))?;
            Ok(Concept::Search {
                subjects,
                pattern: shape.dialect(pattern.clone()),
                fold_case: shape.fold_case,
                descend: shape.descend,
            })
        }
        // ⚠ **The SUBCOMMAND is lost at this level when `--` is used** — a
        // `git log -- p` classifies to [`crate::shell_ops::GitOp::Inspect`],
        // which is the same variant `git show -- p` reaches. So the subcommand
        // is read off `argv`, the one place it survives, exactly as `Page`'s
        // range is. The `Op` is used only for the paths it already resolved.
        Some(Op::Git(git)) => {
            // ⚠ **Every variant that resolved paths, not just `Inspect`.**
            // `git add` reaches `Stage { paths }` and reading only `Inspect`
            // dropped them, so every `git add` refused as `UnreadSubject` —
            // caught by the round-trip test, which is what it is for.
            let paths: &[String] = match git {
                crate::shell_ops::GitOp::Inspect { paths }
                | crate::shell_ops::GitOp::Stage { paths }
                | crate::shell_ops::GitOp::Alter { paths } => paths,
                crate::shell_ops::GitOp::Other { .. } => &[],
            };
            history(step, paths)
        }
        _ => Err(Why::NoLens),
    }
}

/// `git log` — the most recent commits, or the refusal a flag forces.
///
/// ⚠ **Every other subcommand falls to [`Why::NoLens`] and stays in the queue**,
/// where the census ranks it. `status` (10,739 rows), `commit` (9,541) and `add`
/// (9,263) are each their own act and each their own lens; naming them here
/// would be the flattening the vocabulary exists to avoid.
fn history(step: &Step, paths: &[String]) -> Result<Concept, Why> {
    // Decoration: it changes how a commit is printed, never which ones.
    const DECOR: &[&str] = &[
        "--oneline",
        "--no-pager",
        "--abbrev-commit",
        "--date",
        "--color",
        "--no-color",
        "--decorate",
        "--graph",
    ];
    let argv = own_command(step).ok_or(Why::NoLens)?;
    let mut it = argv.iter().map(String::as_str);
    if it.next().map(basename) != Some("git") {
        return Err(Why::NoLens);
    }
    // ⚠ `git -C dir log` — git's own flags come before the subcommand, and `-C`
    // takes a value. The location it names is not a subject; see `History`.
    let mut subcommand = None;
    while let Some(word) = it.next() {
        match word {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                it.next();
            }
            flag if flag.starts_with('-') => {}
            other => {
                subcommand = Some(other);
                break;
            }
        }
    }
    match subcommand {
        Some("log") => {}
        Some("status") => return status(step, paths, it),
        Some("add" | "stage") => return stage(step, paths, it),
        Some("commit") => return commit(it),
        _ => return Err(Why::NoLens),
    }
    let (mut count, mut from, mut after_sep, mut named) = (None, None, false, 0usize);
    let mut rest = it;
    while let Some(word) = rest.next() {
        if word == "--" {
            after_sep = true;
            continue;
        }
        if after_sep {
            named += 1;
            continue;
        }
        let Some(body) = word.strip_prefix('-').filter(|b| !b.is_empty()) else {
            // ⚠ A second revision is a RANGE spelled as two words, and this
            // reader has no way to say that. One is carried; two refuse.
            if from.is_some() {
                return Err(Why::OtherSelection);
            }
            from = Some(word.to_string());
            continue;
        };
        if body.chars().all(|c| c.is_ascii_digit()) {
            count = body.parse().ok();
            continue;
        }
        let base = word.split('=').next().unwrap_or(word);
        if base == "-n" || base == "--max-count" {
            count = match word.split_once('=') {
                Some((_, value)) => value.parse().ok(),
                None => rest.next().and_then(|v| v.parse().ok()),
            };
            if count.is_none() {
                return Err(Why::NoLens);
            }
            continue;
        }
        if DECOR.contains(&base) {
            continue;
        }
        return Err(match base {
            "--all" | "--since" | "--until" | "--before" | "--after" | "--grep" | "-S" | "-G"
            | "--diff-filter" | "--follow" | "--reverse" | "--author" | "--merges"
            | "--no-merges" | "--first-parent" => Why::OtherSelection,
            "--format" | "--pretty" | "-p" | "--patch" | "--stat" | "--name-only"
            | "--name-status" | "--numstat" | "--shortstat" => Why::Formatted,
            _ => Why::NoLens,
        });
    }
    // A path the level below could not resolve would silently narrow the
    // concept to "the whole repository".
    let subjects = counted_subjects(step, paths, named)?;
    Ok(Concept::History {
        count,
        from,
        paths: subjects,
    })
}

/// `ls <dir>` — the entries of a directory, or the refusal its shape forces.
///
/// ⚠ **`find` is turned away here rather than read**, and its own `Why` says
/// why: its operands are a predicate expression, not a locus and a pattern.
///
/// ⚠ **The page reader has already declined this step**, so a `cat`/`head`/
/// `tail`/`sed -n` never reaches here and the two readers cannot both claim one
/// command. Anything else under [`Op::Read`] that is not a listing — `wc`, `du`,
/// `od`, `stat` — falls through to [`Why::NoLens`] and stays in the queue, where
/// the census can rank it.
fn listing(step: &Step, paths: &[String]) -> Result<Concept, Why> {
    let argv = own_command(step).ok_or(Why::NoLens)?;
    match basename(argv.first().ok_or(Why::NoLens)?) {
        "ls" => {}
        "find" | "fd" => return Err(Why::Predicate),
        _ => return Err(Why::NoLens),
    }
    let mut descend = false;
    let mut hidden = false;
    let mut operands = 0;
    for word in argv.iter().skip(1) {
        if word == "--" {
            break;
        }
        if word.starts_with("--") {
            return Err(Why::NoLens);
        }
        let Some(letters) = word.strip_prefix('-').filter(|rest| !rest.is_empty()) else {
            operands += 1;
            continue;
        };
        for letter in letters.chars() {
            match letter {
                'R' => descend = true,
                'a' | 'A' => hidden = true,
                // The `-l` family hands back mode, size and time beside the
                // name — the same locus, a different product.
                'l' | 'h' | 'n' | 'o' | 'g' | 's' | 'i' => return Err(Why::WithMetadata),
                'd' => return Err(Why::NotTheContents),
                _ => return Err(Why::NoLens),
            }
        }
    }
    // ⚠ **`ls` alone is a real locus written implicitly**, and inventing it here
    // is the fabrication this layer refuses. See [`Why::ImplicitLocus`].
    if operands == 0 {
        return Err(Why::ImplicitLocus);
    }
    let loci = counted_subjects(step, paths, operands)?;
    Ok(Concept::List {
        loci,
        descend,
        hidden,
    })
}

/// `wc` — one number per subject, and which number.
///
/// ⚠ **Only `wc`.** `du`, `stat` and `od` reach [`Op::Read`] too and stay in
/// the queue: the first two hand back metadata rather than a reading of the
/// contents, the `stat` shapes carry format strings — the boundary
/// [`Why::Formatted`] names one lens over — and the census sizes all three
/// well below the flags this lens refuses. See [`Concept::Measure`].
///
/// ⚠ **Operands are counted PAST a `--`**, the way `status` and `stage` count
/// and `search_shape` does not (memview#1525): stopping there would drop every
/// subject after it and refuse the row for a miscount.
fn measure(step: &Step, paths: &[String]) -> Result<Concept, Why> {
    let argv = own_command(step).ok_or(Why::NoLens)?;
    if basename(argv.first().ok_or(Why::NoLens)?) != "wc" {
        return Err(Why::NoLens);
    }
    let (mut quantity, mut operands, mut after_sep) = (None, 0usize, false);
    for word in argv.iter().skip(1) {
        if !after_sep && word == "--" {
            after_sep = true;
            continue;
        }
        let flag = (!after_sep)
            .then(|| word.strip_prefix('-').filter(|rest| !rest.is_empty()))
            .flatten();
        let Some(letters) = flag else {
            operands += 1;
            continue;
        };
        for letter in letters.chars() {
            let read = match letter {
                'l' => Quantity::Lines,
                'w' => Quantity::Words,
                'c' => Quantity::Bytes,
                'm' => Quantity::Chars,
                // `-L`, the long forms, `--total` — none made the census.
                _ => return Err(Why::NoLens),
            };
            // Two quantities are a TABLE, a different product — see
            // [`Concept::Measure`]. Queued, where the census can rank it.
            if quantity.replace(read).is_some() {
                return Err(Why::NoLens);
            }
        }
    }
    // Bare `wc` is the POSIX triple, refused for the same reason.
    let quantity = quantity.ok_or(Why::NoLens)?;
    let subjects = counted_subjects(step, paths, operands)?;
    Ok(Concept::Measure { subjects, quantity })
}

/// `git status` — what the tree has that the last commit does not.
///
/// ⚠ **Every flag here is a spelling of one listing.** `--short`, `--porcelain`
/// and `-sb` differ in punctuation and stability, not in which paths appear, so
/// they normalise away. What does NOT is a flag that changes the SET.
fn status<'a>(
    step: &Step,
    paths: &[String],
    rest: impl Iterator<Item = &'a str>,
) -> Result<Concept, Why> {
    let mut named = 0usize;
    let mut after_sep = false;
    for word in rest {
        if word == "--" {
            after_sep = true;
            continue;
        }
        if after_sep {
            named += 1;
            continue;
        }
        let Some(_) = word.strip_prefix('-').filter(|b| !b.is_empty()) else {
            // A path without a `--` — git allows it, but nothing declared it a
            // path, so the guarantee `Inspect` rests on is absent.
            return Err(Why::UnreadSubject);
        };
        let base = word.split('=').next().unwrap_or(word);
        match base {
            "--short" | "-s" | "--porcelain" | "--branch" | "-b" | "-sb" | "--no-color"
            | "--color" | "--long" | "-uno" | "--ahead-behind" => {}
            // A different SET: only staged changes, or a different rule for
            // untracked files.
            "--cached" | "--untracked-files" | "-u" | "--ignored" | "--ignore-submodules" => {
                return Err(Why::OtherSelection);
            }
            _ => return Err(Why::NoLens),
        }
    }
    let subjects = counted_subjects(step, paths, named)?;
    Ok(Concept::Status { paths: subjects })
}

/// `git add` — files put into the index.
fn stage<'a>(
    step: &Step,
    paths: &[String],
    rest: impl Iterator<Item = &'a str>,
) -> Result<Concept, Why> {
    let (mut all, mut operands) = (false, 0usize);
    for word in rest {
        if word == "--" {
            continue;
        }
        let Some(_) = word.strip_prefix('-').filter(|b| !b.is_empty()) else {
            operands += 1;
            continue;
        };
        let base = word.split('=').next().unwrap_or(word);
        match base {
            "-A" | "--all" | "-An" => all = true,
            "-f" | "--force" | "-v" | "--verbose" => {}
            // `-u` stages tracked files only, `-p` is interactive, `-n` and
            // `--dry-run` stage NOTHING — a lowered `Stage` would do what they
            // deliberately did not.
            "-u" | "--update" | "-p" | "--patch" | "-n" | "--dry-run" | "-N"
            | "--intent-to-add" | "-i" | "--interactive" => return Err(Why::OtherSelection),
            _ => return Err(Why::NoLens),
        }
    }
    // ⚠ `git add -A` alone stages the whole repository — a real subject the
    // text never wrote, which is [`Why::ImplicitLocus`] again.
    if operands == 0 {
        return Err(Why::ImplicitLocus);
    }
    let subjects = counted_subjects(step, paths, operands)?;
    Ok(Concept::Stage { subjects, all })
}

/// `git commit` — a commit written, and the message that says why.
///
/// ⚠ **No path guard here.** Staging already happened; a commit writes the
/// repository rather than the operands, and `step.files` is empty for it. The
/// other lenses' `reads_only` check would be asking the wrong question.
fn commit<'a>(rest: impl Iterator<Item = &'a str>) -> Result<Concept, Why> {
    let (mut message, mut amend, mut no_verify, mut from_file) = (None, false, false, false);
    let mut operands = 0usize;
    let mut rest = rest.peekable();
    while let Some(word) = rest.next() {
        if word == "--" {
            continue;
        }
        let Some(_) = word.strip_prefix('-').filter(|b| !b.is_empty()) else {
            operands += 1;
            continue;
        };
        let base = word.split('=').next().unwrap_or(word);
        match base {
            "-m" | "--message" => {
                message = match word.split_once('=') {
                    Some((_, v)) => Some(v.to_string()),
                    None => rest.next().map(str::to_string),
                };
                if message.is_none() {
                    return Err(Why::NoLens);
                }
            }
            // The message exists and is not in this text — a hole, exactly as
            // `sed -i -f fix.sed` is for a substitution.
            "-F" | "--file" => {
                from_file = true;
                rest.next();
            }
            "--amend" => amend = true,
            "--no-verify" | "-n" => no_verify = true,
            "-q" | "--quiet" | "--no-edit" | "-v" | "--verbose" | "-s" | "--signoff" => {}
            // `-a` stages AND commits: two acts, and a lowered `Commit` alone
            // would silently do less.
            "-a" | "--all" | "--reset-author" | "--author" | "--date" | "--fixup" | "--squash"
            | "-C" | "--reuse-message" => return Err(Why::OtherSelection),
            _ => return Err(Why::NoLens),
        }
    }
    // `git commit -- paths` commits only those paths, which this cannot say.
    if operands > 0 {
        return Err(Why::OtherSelection);
    }
    if message.is_none() && !from_file {
        // No `-m` and no `-F`: the message comes from an editor, and nothing in
        // the text or the transcript records what was typed there.
        return Err(Why::NoLens);
    }
    Ok(Concept::Commit {
        message,
        amend,
        no_verify,
    })
}

/// The modifiers a search argv carries, once every flag has been read.
struct Shape {
    extended: bool,
    fixed: bool,
    fold_case: bool,
    descend: bool,
    /// Every word that is not a flag — the pattern, then one per subject.
    operands: usize,
}

impl Shape {
    fn dialect(&self, pattern: String) -> Pattern {
        match (self.fixed, self.extended) {
            (true, _) => Pattern::Fixed(pattern),
            (false, true) => Pattern::Extended(pattern),
            (false, false) => Pattern::Basic(pattern),
        }
    }
}

/// What a search argv asks for beyond its pattern and subjects, or the refusal a
/// flag forces.
///
/// ⚠ **The dialect and the recursion start from the PROGRAM, not from zero.**
/// `egrep` is `grep -E` and `rg` both descends and reads its own dialect by
/// default, so a reader that only looked at flags would call `egrep 'a|b'` basic
/// and lower it to a command matching three literal characters.
///
/// ⚠ **rg's dialect is NOT `-E`.** It is Rust's regex crate — no backreferences,
/// different classes — so it is admitted only where the two agree, which this
/// lens cannot check. `rg` refuses; `ag` and `ack` refuse for the same reason.
/// They are 193 rows against grep's 119,000, and claiming them would be the kind
/// of flattening the whole vocabulary is built to avoid.
///
fn search_shape(step: &Step) -> Result<Shape, Why> {
    let argv = own_command(step).ok_or(Why::NoLens)?;
    let mut shape = match basename(argv.first().ok_or(Why::NoLens)?) {
        "grep" => Shape {
            extended: false,
            fixed: false,
            fold_case: false,
            descend: false,
            operands: 0,
        },
        "egrep" => Shape {
            extended: true,
            fixed: false,
            fold_case: false,
            descend: false,
            operands: 0,
        },
        "fgrep" => Shape {
            extended: false,
            fixed: true,
            fold_case: false,
            descend: false,
            operands: 0,
        },
        _ => return Err(Why::NoLens),
    };
    for word in argv.iter().skip(1) {
        // Everything after `--` is an operand, however it is spelled.
        if word == "--" {
            break;
        }
        if word.starts_with("--") {
            return Err(match word.split('=').next().unwrap_or(word) {
                "--include" | "--exclude" | "--exclude-dir" | "--glob" | "--type" => Why::Filtered,
                "--count"
                | "--files-with-matches"
                | "--files-without-match"
                | "--quiet"
                | "--silent"
                | "--only-matching" => Why::NotLines,
                "--invert-match" => Why::Inverted,
                "--after-context" | "--before-context" | "--context" | "--max-count" => {
                    Why::WithContext
                }
                "--regexp" | "--file" => Why::PatternInFlag,
                _ => Why::NoLens,
            });
        }
        let Some(letters) = word.strip_prefix('-').filter(|rest| !rest.is_empty()) else {
            shape.operands += 1; // an operand, or a bare `-` for stdin
            continue;
        };
        for letter in letters.chars() {
            match letter {
                // ⚠ A digit is a flag's VALUE riding in the same word — `-A6`,
                // `-m1`. The letter it belongs to has already been read, so the
                // digits say nothing more.
                '0'..='9' => {}
                'E' => shape.extended = true,
                'F' => shape.fixed = true,
                'i' => shape.fold_case = true,
                'r' | 'R' => shape.descend = true,
                // Presentational: the same lines, decorated. `-n` numbers them,
                // `-h` drops the filename, `-a` reads binary as text.
                'n' | 'h' | 'H' | 'a' | 's' | 'w' | 'x' => {}
                'c' | 'l' | 'L' | 'q' | 'o' => return Err(Why::NotLines),
                'v' => return Err(Why::Inverted),
                'A' | 'B' | 'C' | 'm' => return Err(Why::WithContext),
                'e' | 'f' => return Err(Why::PatternInFlag),
                _ => return Err(Why::NoLens),
            }
        }
    }
    Ok(shape)
}

/// Build a [`Concept::Page`] once the range is known, applying the refusals
/// every single-command concept shares.
fn page(step: &Step, paths: &[String], range: Range, operands: usize) -> Result<Concept, Why> {
    let subjects = counted_subjects(step, paths, operands)?;
    Ok(Concept::Page { subjects, range })
}

/// The subjects, once the guards every file-reading lens shares have run:
/// nothing touched beyond the operands, every operand resolved, one subject per
/// operand.
///
/// ⚠ **One copy, because the copies drifted.** Both defects this file records —
/// `Page` shipping without the count guard `Search` carried, the git dispatch
/// reading paths from `Inspect` only — were one site missing its copy of a
/// check the others had. A lens that names subjects ends here, or says in a
/// comment why it cannot (see [`commit`]).
///
/// The two checks, and why each exists:
///
/// - **A redirect is a subject the argv never spells.** `head -5 f > out`
///   writes `out` and `grep foo < f` reads an `f` no operand names; both show
///   in `step.files` and neither is in the `Op`'s paths. A concept built from
///   the operands alone would silently do less — gate 2 applied before the
///   fact.
/// - **An operand that produced no subject would read as a stream.** The level
///   below drops a bare word rather than guess — see [`Why::UnreadSubject`] —
///   so the count is the only thing that can tell "acted on a path nobody
///   could resolve" from "acted on what the pipe gave it".
///
/// `operands` is the subject count the argv promises — `None` for a shape that
/// can promise none, which refuses HERE, at the count check, so that the guards
/// above it keep outranking it.
fn counted_subjects(
    step: &Step,
    paths: &[String],
    operands: impl Into<Option<usize>>,
) -> Result<Vec<Subject>, Why> {
    if !reads_only(step, paths) {
        return Err(Why::NoLens);
    }
    let subjects = subjects_or_refuse(step, paths)?;
    if operands.into() != Some(subjects.len()) {
        return Err(Why::UnreadSubject);
    }
    Ok(subjects)
}

/// Does this step read exactly the operands the op named, and write nothing?
///
/// The first guard of [`counted_subjects`], which is its only caller — named
/// separately because it answers a different question (about `step.files`) than
/// the subject checks below it.
fn reads_only(step: &Step, paths: &[String]) -> bool {
    !step.files.iter().any(|use_| use_.write)
        && !step
            .files
            .iter()
            .any(|use_| !use_.write && !paths.contains(&use_.path))
}

/// The subjects, or the refusal their kind forces — the checks `Rewrite` and
/// `Page` share, in the order that lets the cheapest win.
fn subjects_or_refuse(step: &Step, paths: &[String]) -> Result<Vec<Subject>, Why> {
    // ⚠ **A remote step's files are never local**, and a lowered local command
    // would claim work on the wrong machine. The step says so; `files` is empty
    // for them by construction, which would otherwise look like a command that
    // named nothing.
    if step.host.is_some() {
        return Err(Why::Remote);
    }
    let subjects = subjects(step, paths);
    // ⚠ **A DESCRIBED subject cannot be lowered, so it is refused by name.**
    // `Bounded` and `Located` are the reader's middle — `S ⊆ L` at a locus — and
    // no single command spells them: measured 2026-09-03, lowering
    // `/home/…/*.ts` and lifting it back gives [`Subject::Named`], because a
    // pattern written literally in an operand position IS a resolved path to
    // this reader. The language came from a loop, and a loop is not what a
    // single-command concept lowers to.
    //
    // Silently keeping them would be worse than dropping them: it turns a
    // described middle into a **false lower bound**, which is the one direction
    // this whole reader refuses. So the lens accepts `Named` and `Hole`, and
    // what it cannot express stays an L2/L3 leaf and is counted — refuse rather
    // than mis-model, the same rule the grammar follows.
    if subjects
        .iter()
        .any(|s| matches!(s, Subject::Bounded(_) | Subject::Located(_)))
    {
        return Err(Why::Described);
    }
    Ok(subjects)
}

/// The step's own command with its wrappers unwrapped — or `None` when `xargs`
/// is among them.
///
/// ⚠ **`argv` keeps the wrappers, and `xargs` is the one that must not be
/// unwrapped away.** `xargs head -5` pages the files a pipe supplies —
/// subjects no operand names — and after unwrapping it is indistinguishable
/// from a stream `head -5`. So it is refused before the command is read, the
/// same reason a redirect is.
///
/// ⚠ **One copy, for the same reason [`counted_subjects`] is** — this lived as
/// three, and the git family had none, so `xargs git log` lifted to a
/// whole-repository `History` about paths only the pipe knew.
fn own_command(step: &Step) -> Option<&[String]> {
    let argv = unwrap_command(&step.argv);
    let wrappers = &step.argv[..step.argv.len() - argv.len()];
    if wrappers.iter().any(|w| basename(w) == "xargs") {
        return None;
    }
    Some(argv)
}

/// The range a `head`/`tail`/`cat` step shows, or `None` if this read is not a
/// page the lens accepts.
fn read_page(step: &Step) -> Option<(Range, usize)> {
    let argv = own_command(step)?;
    match basename(argv.first()?) {
        // ⚠ **`cat` with any flag is not a bare page** — `cat -n` numbers its
        // output, `cat -A` shows control characters; both change what is seen.
        "cat" => flagless(argv).then(|| (Range::All, plain_operands(argv))),
        "head" => line_count(argv).map(|(n, operands)| (Range::First(n), operands)),
        "tail" => line_count(argv).map(|(n, operands)| (Range::Last(n), operands)),
        _ => None,
    }
}

/// Every word after the command that is not a flag, and not the stdin `-`.
///
/// ⚠ **Only sound where no accepted flag takes a SEPARATE value**, which is why
/// `head` and `tail` count inside [`line_count`] instead: `head -n 5 f` would
/// read the `5` as an operand here and call the page a two-subject one.
fn plain_operands(argv: &[String]) -> usize {
    argv.iter()
        .skip(1)
        .filter(|word| !word.starts_with('-'))
        .count()
}

/// The span a `sed -n 'a,bp'` shows, or `None` for any other sed program.
///
/// ⚠ **`-n` is required and is the whole difference.** Without it `sed '1,5p'`
/// prints the file AND lines 1-5 again — a different output, so it must not
/// read as a page. A `$`-relative address (`1,$p`), a `d`elete, or a
/// substitution all fail the digit parse and refuse.
fn sed_page(step: &Step, program: &str) -> Option<(Range, usize)> {
    let argv = unwrap_command(&step.argv);
    if !argv.iter().any(|w| w == "-n") {
        return None;
    }
    let body = program.strip_suffix('p')?;
    match body.split_once(',') {
        Some((a, b)) => {
            let (a, b) = (a.parse().ok()?, b.parse().ok()?);
            let range = if a == 1 {
                Range::First(b)
            } else {
                Range::Lines(a, b)
            };
            // ⚠ The sed PROGRAM is an operand as well as the files, so one is
            // subtracted here rather than in `page`, which must not need to know
            // which spelling it was handed.
            Some((range, plain_operands(argv).checked_sub(1)?))
        }
        None => {
            let n = body.parse().ok()?;
            Some((Range::Lines(n, n), plain_operands(argv).checked_sub(1)?))
        }
    }
}

/// The line count a `head`/`tail` argv asks for — its `-N`, `-n N` or default
/// ten — or `None` if a flag makes it something other than a line page.
///
/// ⚠ **The default is POSIX's, a documented fact and not a guess**, so the
/// concept carries the number the text left implicit and the round trip holds.
/// A byte count (`-c`), a follow (`-f`), a `+N` prefix drop, or any flag this
/// does not name refuses — each is a different act the lowered form could not
/// honour.
fn line_count(argv: &[String]) -> Option<(u32, usize)> {
    let mut count = None;
    let mut operands = 0;
    let mut i = 1;
    while i < argv.len() {
        let Some(rest) = argv[i].strip_prefix('-') else {
            operands += 1; // an operand — a path
            i += 1;
            continue;
        };
        if rest.is_empty() {
            i += 1; // a bare `-`, stdin — named, but not a file to resolve
            continue;
        }
        if rest.chars().all(|c| c.is_ascii_digit()) {
            count = Some(digits(rest)?); // -5
            i += 1;
            continue;
        }
        if argv[i] == "-n" || argv[i] == "--lines" {
            count = Some(digits(argv.get(i + 1)?)?);
            i += 2;
            continue;
        }
        if let Some(v) = rest.strip_prefix('n').filter(|v| !v.is_empty()) {
            count = Some(digits(v)?); // -n5
            i += 1;
            continue;
        }
        return None; // -c, -f, -q, anything else
    }
    Some((count.unwrap_or(10), operands))
}

/// A plain unsigned count, or `None`.
///
/// ⚠ **`u32::parse` accepts a leading `+`, and `tail -n +2` means the
/// opposite of a count** — it drops the first line and shows the rest, so
/// `"+2".parse()` reading as `2` lifted a prefix-drop as a two-line tail
/// (caught by the refusal test, 2026-09-04). Digits only.
fn digits(word: &str) -> Option<u32> {
    word.chars()
        .all(|c| c.is_ascii_digit())
        .then(|| word.parse().ok())
        .flatten()
}

/// Whether a command carries no flags — only `-` (stdin) and operands.
fn flagless(argv: &[String]) -> bool {
    !argv
        .iter()
        .skip(1)
        .any(|w| w.starts_with('-') && w.len() > 1)
}

/// The subjects, read straight off the step's four accounts.
///
/// ⚠ **`Op::Transform.paths` holds only what RESOLVED**, which is the whole
/// reason this cannot be built from the operation alone: `sed -i 's/a/b/'
/// "$TARGET"` arrives with `paths: []` and the subject in `step.unnamed`, and a
/// lift reading `paths` would report a rewrite of nothing at all. Measured, not
/// assumed — the first version of this did exactly that and the acceptance test
/// for holes caught it.
///
/// ⚠ **Read off the accounts, never re-derived.** The first version matched
/// resolved paths back to their words by comparing leaves, which is a second
/// implementation of resolution and would disagree **silently** the moment a
/// `cd`, a loop variable or a `~` made the word and the path differ — the same
/// argument `shell_files::trace` is built on. The accounts already say which of
/// the three kinds each subject is; this reads them in that order.
///
/// The order is named, described, then counted — the reader's own three-part
/// artefact, so two occurrences of one shape produce the same list.
fn subjects(step: &Step, paths: &[String]) -> Vec<Subject> {
    let mut out: Vec<Subject> = paths.iter().cloned().map(Subject::Named).collect();
    out.extend(step.bounded.iter().cloned().map(Subject::Bounded));
    out.extend(step.located.iter().cloned().map(Subject::Located));
    // Counted, not named: one hole per admission, so a command that could not
    // name two subjects does not lower as though it had one.
    out.extend(step.unnamed.iter().map(|_| Subject::Hole));
    out
}

/// Turn a concept back into a command that does the same thing.
///
/// ⚠ **One canonical spelling, chosen the way the printer chooses quoting.**
/// `sed -i` stands for every in-place transform the lift accepts, so
/// `perl -pi -e` lowers to `sed -i` and the original language survives as
/// provenance rather than as structure — which is what "language choice is
/// spelling" means at this level, and why `lower ∘ lift` is permitted to differ
/// from the text it started from.
///
/// ⚠ **A hole lowers to an unexpanded variable, and that is what a hole IS.**
/// The losslessness claim is that the same holes come back, not fewer — so the
/// spelling has to be one this reader reads back as unnamed. `?` is not:
/// measured, it carries no `/` and no extension, so [`crate::shell_ops::looks_like_path`] refuses
/// it and the subject vanishes entirely, which fails the law. `"$UNNAMED"` is
/// recorded as an admission and lifts back to [`Subject::Hole`].
///
/// ⚠ **An earlier version of this comment said the lowered form was "text a
/// shell would not run, and meant not to be".** That contradicted gate 3, which
/// requires the lowered text to be valid shell, and the law refuted it: a
/// spelling nothing reads back cannot round-trip. What keeps a lowered concept
/// from being mistaken for a script is that its holes are unbound, so a shell
/// given one fails rather than doing something else.
pub fn lower(concept: &Concept) -> String {
    match concept {
        Concept::Rewrite {
            subjects,
            substitution,
        } => {
            let program = substitution.as_deref().unwrap_or("?");
            let words: Vec<String> = subjects.iter().map(spell).collect();
            format!("sed -i '{program}' {}", words.join(" "))
        }
        // ⚠ **One canonical spelling per range, and `sed -n` is it for a span** —
        // the same rule `Rewrite` follows in picking `sed -i`. `head -5` and
        // `sed -n '1,5p'` both lift to `First(5)`, and both lower to `head -5`:
        // the spelling normalises and the concept is what survives.
        Concept::Page { subjects, range } => {
            let words: Vec<String> = subjects.iter().map(spell).collect();
            let head = match range {
                Range::All => "cat".to_string(),
                Range::First(n) => format!("head -{n}"),
                Range::Last(n) => format!("tail -{n}"),
                Range::Lines(a, b) => format!("sed -n '{a},{b}p'"),
            };
            // A stream page names no file — `head -50` alone. Kept tidy so the
            // lowered text is what the reader reads back, trailing space and all.
            if words.is_empty() {
                head
            } else {
                format!("{head} {}", words.join(" "))
            }
        }
        // ⚠ **`grep` is the canonical spelling and the flags are rebuilt from
        // the fields, not remembered.** `egrep foo` and `grep -E foo` both lift
        // to `Extended` and both lower to `grep -E`; that is the same
        // normalisation `sed -i` performs for `perl -pi`. The order is fixed so
        // the lowered text is a function of the concept alone.
        Concept::Search {
            subjects,
            pattern,
            fold_case,
            descend,
        } => {
            let (dialect, text) = match pattern {
                Pattern::Basic(text) => ("", text),
                Pattern::Extended(text) => (" -E", text),
                Pattern::Fixed(text) => (" -F", text),
            };
            let mut head = format!("grep{dialect}");
            if *fold_case {
                head.push_str(" -i");
            }
            if *descend {
                head.push_str(" -r");
            }
            // ⚠ Quoted, always. A pattern is a regular expression and holds `|`,
            // `*` and spaces; unquoted it would be read back as several operands
            // or expanded by the shell, and the law would catch neither because
            // both would still parse.
            let head = format!("{head} '{text}'");
            if subjects.is_empty() {
                head
            } else {
                format!(
                    "{head} {}",
                    subjects.iter().map(spell).collect::<Vec<_>>().join(" ")
                )
            }
        }
        // ⚠ **A locus is always written**, because the lift refuses the
        // no-operand form outright — see [`Why::ImplicitLocus`]. So there is no
        // subjectless branch here, unlike `Page` and `Search`.
        // ⚠ `--short` is not written back for the same reason `--oneline` is
        // not: it chooses a spelling of one listing.
        Concept::Status { paths } => match paths.is_empty() {
            true => "git status".to_string(),
            false => format!(
                "git status -- {}",
                paths.iter().map(spell).collect::<Vec<_>>().join(" ")
            ),
        },
        Concept::Stage { subjects, all } => {
            let flag = if *all { " -A" } else { "" };
            format!(
                "git add{flag} {}",
                subjects.iter().map(spell).collect::<Vec<_>>().join(" ")
            )
        }
        // ⚠ A message that was never in the text lowers to `-F -`, which reads
        // back as the same hole. Spelling it `-m ''` would invent an empty
        // message, and an empty message is a different commit.
        Concept::Commit {
            message,
            amend,
            no_verify,
        } => {
            let mut out = "git commit".to_string();
            if *amend {
                out.push_str(" --amend");
            }
            if *no_verify {
                out.push_str(" --no-verify");
            }
            match message {
                Some(text) => out.push_str(&format!(" -m '{text}'")),
                None => out.push_str(" -F -"),
            }
            out
        }
        // ⚠ `--oneline` is not written back: it is decoration, and the lift
        // normalises it away like quoting. What must survive is the count, the
        // revision and the paths, because each changes WHICH commits appear.
        Concept::History { count, from, paths } => {
            let mut out = "git log".to_string();
            if let Some(n) = count {
                out.push_str(&format!(" -{n}"));
            }
            if let Some(rev) = from {
                out.push_str(&format!(" {rev}"));
            }
            if !paths.is_empty() {
                out.push_str(" -- ");
                out.push_str(&paths.iter().map(spell).collect::<Vec<_>>().join(" "));
            }
            out
        }
        Concept::List {
            loci,
            descend,
            hidden,
        } => {
            let mut head = "ls".to_string();
            if *descend {
                head.push_str(" -R");
            }
            if *hidden {
                head.push_str(" -a");
            }
            format!(
                "{head} {}",
                loci.iter().map(spell).collect::<Vec<_>>().join(" ")
            )
        }
        // The flag is rebuilt from the field, and `-c` is the canonical
        // spelling for bytes — the same normalisation `grep -E` gets.
        Concept::Measure { subjects, quantity } => {
            let head = match quantity {
                Quantity::Lines => "wc -l",
                Quantity::Words => "wc -w",
                Quantity::Bytes => "wc -c",
                Quantity::Chars => "wc -m",
            };
            // A stream measure names no file — `wc -l` alone, as `Page` does.
            if subjects.is_empty() {
                head.to_string()
            } else {
                format!(
                    "{head} {}",
                    subjects.iter().map(spell).collect::<Vec<_>>().join(" ")
                )
            }
        }
    }
}

/// The concept as a sentence a person reads.
///
/// ⚠ **A phrase, not the lowered command.** [`lower`] answers the law and so
/// prints canonical shell; someone approving a command already has the command,
/// and what argv does not say is what it was FOR — which is why
/// `docs/concept-model.md` makes the ask card the first consumer, and why every
/// payload trap in the corpus is a spelling that hides the act.
///
/// ⚠ **This is the CONCEPT's phrase, not any one surface's.** A view building
/// its own would be a second reading, free to drift from the one the gates
/// hold. A surface too narrow for the whole sentence shortens what this returns
/// rather than rewording it — at phone width the resolved path is already
/// carried by the use row and the sheet's footer (memview#1454).
///
/// ⚠ **A hole must READ as a hole.** `"$UNNAMED"` is right for the lowered
/// text, which has to read back as an admission; on a card it looks like a
/// variable somebody could go and check.
pub fn describe(concept: &Concept) -> String {
    match concept {
        Concept::Rewrite { subjects, .. } => {
            format!("Rewrite {} in place", said(subjects))
        }
        Concept::Page { subjects, range } => {
            let part = match range {
                Range::All => "all of".to_string(),
                Range::First(1) => "the first line of".to_string(),
                Range::Last(1) => "the last line of".to_string(),
                Range::First(n) => format!("the first {n} lines of"),
                Range::Last(n) => format!("the last {n} lines of"),
                Range::Lines(a, b) if a == b => format!("line {a} of"),
                Range::Lines(a, b) => format!("lines {a} to {b} of"),
            };
            // A stream page named no file — `… | head -50`. Saying "what it is
            // given" rather than inventing a subject is the same refusal the
            // lens makes when it declines to read a redirect's target as an
            // operand.
            if subjects.is_empty() {
                format!("Show {part} what it is given")
            } else {
                format!("Show {part} {}", said(subjects))
            }
        }
        Concept::Search {
            subjects,
            pattern,
            fold_case,
            descend,
        } => {
            let (text, how) = match pattern {
                Pattern::Basic(text) => (text, ""),
                Pattern::Extended(text) => (text, ""),
                // ⚠ Worth saying, because it is the case where the punctuation
                // in the pattern means nothing: `*` is an asterisk, not a
                // repeat. A card that read it as a regex would mislead.
                Pattern::Fixed(text) => (text, ", as a literal string"),
            };
            let case = if *fold_case { ", ignoring case" } else { "" };
            let where_ = match (subjects.is_empty(), descend) {
                (true, _) => "what it is given".to_string(),
                (false, true) => format!("everything under {}", said(subjects)),
                (false, false) => said(subjects),
            };
            format!("Find lines matching {text} in {where_}{case}{how}")
        }
        Concept::Status { paths } => match paths.is_empty() {
            true => "Show what the working tree has that the last commit does not".to_string(),
            false => format!("Show what has changed in {}", said(paths)),
        },
        // ⚠ Says STAGE, never "write" — the level below is explicit that this
        // changes no file, and a card that read it as a write would be wrong
        // about the one thing approval is for.
        Concept::Stage { subjects, all } => {
            let how = if *all { ", deletions included" } else { "" };
            format!("Stage {}{how}", said(subjects))
        }
        Concept::Commit {
            message,
            amend,
            no_verify,
        } => {
            let what = if *amend {
                "Amend the last commit"
            } else {
                "Commit the staged changes"
            };
            let saying = match message {
                Some(text) => format!(" saying \"{text}\""),
                None => " with a message this command does not carry".to_string(),
            };
            // ⚠ Named outright: skipping the hooks is the thing an approver most
            // needs told, and it is invisible in a tidy summary.
            let gate = if *no_verify {
                " — SKIPPING the pre-commit gate"
            } else {
                ""
            };
            format!("{what}{saying}{gate}")
        }
        Concept::History { count, from, paths } => {
            let how_many = match count {
                Some(1) => "the last commit".to_string(),
                Some(n) => format!("the last {n} commits"),
                // git's own default is unbounded, and saying so beats inventing
                // a number the text never gave.
                None => "the commit history".to_string(),
            };
            let of = if paths.is_empty() {
                String::new()
            } else {
                format!(" touching {}", said(paths))
            };
            let at = match from {
                Some(rev) => format!(" from {rev}"),
                None => String::new(),
            };
            format!("Show {how_many}{at}{of}")
        }
        Concept::List {
            loci,
            descend,
            hidden,
        } => {
            let what = if *descend {
                "List everything under"
            } else {
                "List the entries of"
            };
            // ⚠ Said plainly, because a hidden entry is the one a person
            // scanning an approval would not otherwise expect to be touched.
            let also = if *hidden {
                ", hidden ones included"
            } else {
                ""
            };
            format!("{what} {}{also}", said(loci))
        }
        Concept::Measure { subjects, quantity } => {
            let what = match quantity {
                Quantity::Lines => "lines",
                Quantity::Words => "words",
                // ⚠ Bytes, not characters — the card must not soften the one
                // distinction the flag pair exists to draw.
                Quantity::Bytes => "bytes",
                Quantity::Chars => "characters",
            };
            if subjects.is_empty() {
                format!("Count the {what} of what it is given")
            } else {
                format!("Count the {what} of {}", said(subjects))
            }
        }
    }
}

/// The subjects as a phrase, every one of them.
///
/// ⚠ **Never a summary.** "2 files" would let the card claim a concept while
/// hiding which files, which is the one thing approval is for.
///
/// ⚠ **The `Bounded` and `Located` arms are UNREACHABLE and therefore
/// unverified.** [`subjects_or_refuse`] turns both into [`Why::Described`]
/// before a `Concept` exists, so no test covers them and no corpus command has
/// ever produced one. They are kept against that refusal being lifted, and
/// named here so they are not mistaken for exercised code.
fn said(subjects: &[Subject]) -> String {
    subjects
        .iter()
        .map(|subject| match subject {
            Subject::Named(path) => path.clone(),
            Subject::Bounded(pattern) => format!("a file matching {pattern}"),
            Subject::Located(locus) => format!("a file under {locus}"),
            Subject::Hole => "a file this command does not name".to_string(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A subject as the lowered text writes it.
///
/// ⚠ **`Bounded` and `Located` are UNREACHABLE here too, for the reason
/// [`said`] gives** — [`subjects_or_refuse`] turns both into [`Why::Described`]
/// before a `Concept` exists. Said in both places because they are two
/// functions: a reader meeting this one alone has nothing to tell it that these
/// arms are unexercised, and they are phrased as confidently as the live ones.
fn spell(subject: &Subject) -> String {
    match subject {
        Subject::Named(path) => path.clone(),
        // The pattern it is a subset of, which is what the reader knows and all
        // it knows: `⟦*.log⟧ = some S ⊆ L(*.log)`.
        Subject::Bounded(pattern) => pattern.clone(),
        // Rooted at this directory, with the leaf unknown.
        Subject::Located(locus) => format!("{}/?", locus.trim_end_matches('/')),
        Subject::Hole => "\"$UNNAMED\"".to_string(),
    }
}
