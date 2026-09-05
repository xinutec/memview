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
            Some(range) => page(step, paths, range),
            None => Err(Why::NotInPlace),
        },
        // ⚠ **A read is a page only for the four pagers, and only in the
        // shapes the corpus spells.** `wc -l`, `ls`, `od` also reach
        // [`Op::Read`]; they measure or list rather than show, so they stay
        // counted leaves. The range comes from `argv`, since the projection
        // dropped it.
        Some(Op::Read { paths }) => match read_page(step) {
            Some(range) => page(step, paths, range),
            None => Err(Why::NoLens),
        },
        _ => Err(Why::NoLens),
    }
}

/// Build a [`Concept::Page`] once the range is known, applying the refusals
/// every single-command concept shares.
fn page(step: &Step, paths: &[String], range: Range) -> Result<Concept, Why> {
    // ⚠ **A page reads its operands and writes nothing — so a redirect is a
    // subject the argv never spells.** `head -5 f > out` writes `out` and
    // `head -5 < f` reads an `f` no operand names; both show in `step.files`
    // but not in the `Op`'s paths. A lowered `Page` built from the operands
    // alone would silently do less, which is gate 2 applied before the fact.
    if step.files.iter().any(|use_| use_.write)
        || step
            .files
            .iter()
            .any(|use_| !use_.write && !paths.contains(&use_.path))
    {
        return Err(Why::NoLens);
    }
    let subjects = subjects_or_refuse(step, paths)?;
    Ok(Concept::Page { subjects, range })
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

/// The range a `head`/`tail`/`cat` step shows, or `None` if this read is not a
/// page the lens accepts.
///
/// ⚠ **`argv` keeps the wrappers, and `xargs` is the one that must not be
/// unwrapped away here.** `xargs head -5` pages the files a pipe supplies —
/// subjects no operand names — and after unwrapping it is indistinguishable
/// from a stream `head -5`. So it is refused before the command is read, the
/// same reason a redirect is.
fn read_page(step: &Step) -> Option<Range> {
    let argv = unwrap_command(&step.argv);
    if step.argv.len() > argv.len()
        && step.argv[..step.argv.len() - argv.len()]
            .iter()
            .any(|w| basename(w) == "xargs")
    {
        return None;
    }
    match basename(argv.first()?) {
        // ⚠ **`cat` with any flag is not a bare page** — `cat -n` numbers its
        // output, `cat -A` shows control characters; both change what is seen.
        "cat" => flagless(argv).then_some(Range::All),
        "head" => line_count(argv).map(Range::First),
        "tail" => line_count(argv).map(Range::Last),
        _ => None,
    }
}

/// The span a `sed -n 'a,bp'` shows, or `None` for any other sed program.
///
/// ⚠ **`-n` is required and is the whole difference.** Without it `sed '1,5p'`
/// prints the file AND lines 1-5 again — a different output, so it must not
/// read as a page. A `$`-relative address (`1,$p`), a `d`elete, or a
/// substitution all fail the digit parse and refuse.
fn sed_page(step: &Step, program: &str) -> Option<Range> {
    let argv = unwrap_command(&step.argv);
    if !argv.iter().any(|w| w == "-n") {
        return None;
    }
    let body = program.strip_suffix('p')?;
    match body.split_once(',') {
        Some((a, b)) => {
            let (a, b) = (a.parse().ok()?, b.parse().ok()?);
            Some(if a == 1 {
                Range::First(b)
            } else {
                Range::Lines(a, b)
            })
        }
        None => {
            let n = body.parse().ok()?;
            Some(Range::Lines(n, n))
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
fn line_count(argv: &[String]) -> Option<u32> {
    let mut count = None;
    let mut i = 1;
    while i < argv.len() {
        let Some(rest) = argv[i].strip_prefix('-') else {
            i += 1; // an operand — a path
            continue;
        };
        if rest.is_empty() {
            i += 1; // a bare `-`, stdin
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
    Some(count.unwrap_or(10))
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
    }
}

/// The concept as a sentence a person reads, for the ask card.
///
/// ⚠ **A phrase, not the lowered command, and they are different jobs.**
/// [`lower`] answers the law — it must read back as this concept — so it is
/// held to a canonical spelling and prints `sed -n '3,7p' x`. A person
/// approving a command already has the command; what the card owes them is the
/// thing argv does not say, which is what it is FOR. `docs/concept-model.md`
/// names that as the first consumer: approval today reads *spelling*, and every
/// payload trap in the corpus is a spelling that hides the act.
///
/// ⚠ **It lives here rather than in the console, for the reason `parse.rs`
/// already follows**: a phrase built by the view would be a second reading of
/// the concept, free to drift from the one the gates hold. The console renders
/// what this returns.
///
/// ⚠ **A hole must READ as a hole.** `"$UNNAMED"` is the right spelling for the
/// lowered text, because the reader has to read it back as an admission; on a
/// card it would look like a variable a person could go and check. So it says
/// so in words, and the card is then honest about the one thing that matters
/// most: this command touches a file whose name is not in it.
pub fn describe(concept: &Concept) -> String {
    match concept {
        Concept::Rewrite { subjects, .. } => {
            format!("Rewrite {} in place", named(subjects))
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
                format!("Show {part} {}", named(subjects))
            }
        }
    }
}

/// The subjects as a phrase, every one of them.
///
/// ⚠ **Never a summary.** "2 files" would let the card claim a concept while
/// hiding which files, which is the one thing approval is for.
fn named(subjects: &[Subject]) -> String {
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
