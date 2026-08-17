//! Did the reader predict what the shell actually did?
//!
//! Everything else in this crate measures how *much* is read: 98.4% of simple
//! commands understood, 95.9% of file uses named. None of it asks whether what
//! was read is **right**, and a wrong answer given confidently is worse than a
//! gap admitted. This is the only test that can catch one.
//!
//! ⚠ **The one test in this crate that runs a program**, and the crate doc makes
//! a point of saying nothing here runs one. That claim is about the library and
//! the two binaries that link it; this is a dev-time harness, like the
//! tree-sitter probe beside it, and nothing links it into anything. It executes
//! only the fixtures written in this file, in a scratch directory it makes and
//! removes. **No corpus command is ever re-executed** — the corpus is history,
//! and running it would be both dangerous and meaningless.
//!
//! # How the truth is obtained
//!
//! A directory of shims goes first on `PATH`. Each one records the arguments it
//! was handed and the directory it was handed them in, then execs the real tool.
//! So the log is not a guess about what bash would do — it is what bash did:
//! globs expanded against the day's filesystem, variables substituted, loops
//! iterated, quoting resolved.
//!
//! ⚠ **`cd` is a builtin and cannot be shimmed, which is why the directory is
//! logged rather than the move.** That turns out to be the better measurement:
//! what matters is not that `cd src` was seen but that the `wc` after it ran in
//! `src`, and every relative path in the corpus hangs off exactly that. 48,172
//! `cd` operations rest on this and nothing had ever checked one.
//!
//! # The two properties
//!
//! **Sound** — the reader never predicts a command the shell did not run. This
//! holds for every fixture, including those whose values the text does not
//! determine: predicting *less* than happened is an undercount, predicting
//! something that never happened is a fabrication, and only the second is a lie.
//!
//! **Exact where determined** — where the text determines everything, the
//! prediction is *equal* to the log, not merely contained in it. A reader that
//! passed the first property by predicting nothing would fail this one.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A home directory for the fixtures, which is not this machine's.
const HOME: &str = "/home/example";

/// The verbs given shims. Every one is in the reader's own table, because a
/// command it does not model is not a command it can be wrong about.
///
/// `bash` is deliberately absent: shimming it would log `bash -c '…'` as one
/// entry and hide the commands inside, which are the ones worth comparing.
const SHIMMED: &[&str] = &[
    "cat", "wc", "grep", "sed", "cp", "mv", "rm", "touch", "head", "tail", "tee", "ls",
];

/// Between two arguments in the log, so that an argument containing a space is
/// still one argument. A tab or a space would not survive `wc -l "my file"`.
const SEP: char = '\u{1f}';

/// A scratch directory that removes itself, so a failing test leaves no litter
/// and a passing one leaves no state for the next.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One command as it really ran: where it ran, and what it was given.
type Ran = (String, Vec<String>);

/// One command as the reader claims it, with the confidence it claims it at.
///
/// ⚠ **Soundness is a property of the *certain* claims, not of all of them.** A
/// command the reader records as [`Reached::Sometimes`] is a statement about the
/// script — "this is in the text, under a condition" — and the shell not running
/// it refutes nothing. Comparing without the confidence would make both arms of
/// an `if` unfalsifiable, since one of them never runs by construction.
type Claim = (reader::shell::Reached, Ran);

/// What a prediction that could not be checked was set aside for.
#[derive(Debug, Default, PartialEq, Eq)]
struct Aside {
    /// Recorded with a word the text does not determine — a folded loop body.
    admitted: usize,
    /// Recorded under a condition, so the shell was free not to run it.
    uncertain: usize,
}

/// Run `script` for real and return what the shimmed commands received.
///
/// `given` is the tree the fixture starts from — `(relative path, contents)` —
/// because a glob has nothing to expand against otherwise, and "what the day's
/// filesystem answered" is the whole point of running it at all.
fn actually(script: &str, given: &[(&str, &str)]) -> (Scratch, Vec<Ran>) {
    let root = std::env::temp_dir().join(format!(
        "reader-oracle-{}-{}",
        std::process::id(),
        // Distinct per fixture within one test run, without a clock or a random
        // source: the script's own length and first bytes are enough here and
        // keep the directory name reproducible when a failure is investigated.
        script.len()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("a scratch directory");
    // ⚠ Canonical, or the comparison fails on macOS for a reason that has
    // nothing to do with the reader: `/var/folders/…` is a symlink to
    // `/private/var/folders/…`, and bash reports the resolved one in `PWD`.
    let root = std::fs::canonicalize(&root).expect("a canonical scratch directory");
    let scratch = Scratch(root.clone());

    for (path, contents) in given {
        let at = root.join(path);
        if let Some(parent) = at.parent() {
            std::fs::create_dir_all(parent).expect("a fixture directory");
        }
        std::fs::write(&at, contents).expect("a fixture file");
    }

    let log = root.join("oracle.log");
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).expect("a shim directory");
    for verb in SHIMMED {
        // Resolved from the ambient PATH *before* the shims are in front of it,
        // so a shim execs the real tool rather than itself.
        let Some(real) = which(verb) else { continue };
        let shim = format!(
            "#!/bin/sh\n\
             printf '%s' \"$PWD\" >> {log}\n\
             printf '{sep}%s' '{verb}' >> {log}\n\
             for a in \"$@\"; do printf '{sep}%s' \"$a\" >> {log}; done\n\
             printf '\\n' >> {log}\n\
             exec {real} \"$@\"\n",
            log = log.display(),
            real = real.display(),
            verb = verb,
            sep = SEP,
        );
        let at = bin.join(verb);
        std::fs::write(&at, shim).expect("a shim");
        make_runnable(&at);
    }

    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let done = Command::new("bash")
        .arg("-c")
        .arg(script)
        .current_dir(&root)
        .env("PATH", path)
        .env("HOME", HOME)
        .output()
        .expect("bash runs the fixture");
    assert!(
        done.status.success(),
        "the fixture itself failed, so there is nothing to compare:\n{}",
        String::from_utf8_lossy(&done.stderr)
    );

    let text = std::fs::read_to_string(&log).unwrap_or_default();
    let ran = text
        .lines()
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.split(SEP);
            let cwd = fields.next().unwrap_or_default().to_string();
            (cwd, fields.map(str::to_string).collect())
        })
        .collect();
    (scratch, ran)
}

/// What the reader says the same script did, in the same shape.
///
/// Only the shimmed verbs, because only those can be compared — a wrapper's own
/// step has no counterpart in the log by design.
fn predicted(script: &str, root: &Path) -> Vec<Claim> {
    let cmds = reader::project::read(script)
        .unwrap_or_else(|at| panic!("the fixture does not parse, stopped at {at:?}"));
    let root = root.to_string_lossy().to_string();
    reader::shell_files::trace(&cmds, Some(&root), HOME)
        .steps
        .into_iter()
        // ⚠ **Compared after `unwrap_command`, and the first run of this test is
        // why.** A step's `argv` is documented as "the words as the shell would
        // have run them", and for a loop body it is `["do", "wc", "-l",
        // "a.log"]` — the keyword is still on the front. Nothing runs `do`. The
        // reader classifies through `unwrap_command`, so that is the form it
        // actually believes in, and comparing anything else would test how the
        // record is spelled rather than whether the reading is right.
        //
        // The spelling still matters to whoever displays a step beside the claim
        // it supports (memview#93), which is filed separately — it is a
        // different question from this one.
        .filter_map(|step| {
            let argv: Vec<String> = reader::shell_ops::unwrap_command(&step.argv).to_vec();
            let head = argv.first()?;
            SHIMMED
                .contains(&reader::shell_ops::basename(head))
                .then(|| (step.reached, (step.cwd.unwrap_or_default(), argv)))
        })
        .collect()
}

/// Whether a predicted command still carries a word the text did not determine.
///
/// ⚠ **The distinction the first run of this file forced.** For a folded loop the
/// reader records the body once, as written — `wc -l $f` — and that command never
/// ran in that form: bash ran `wc -l a.log` three times. Read as a claim about an
/// execution it is false; read as what it is, *the script contains this command
/// and what it ran as is not determined*, it is the honest record and the same
/// `$f` is counted among the subjects the reader cannot name.
///
/// So a step like this is neither confirmed nor a fabrication, and lumping it in
/// with either would make the property below untestable. It is set aside and
/// counted. When memview#819 turns `$f` into a bounded set, these are the steps
/// that acquire a checkable claim.
fn admitted_unknown(argv: &[String]) -> bool {
    argv.iter().any(|word| word.contains(['$', '`', '*', '?']))
}

/// The reader never claims a *determinate* command that did not happen.
///
/// Stated as multiset containment rather than order: a `for` run out and the
/// same loop run by bash agree on what ran, and this test is not the place to
/// argue about the order two independent walks emit them in.
///
/// Returns what was set aside and why, so a test can say what it expected to be
/// unresolved rather than letting a growing pile of them pass unnoticed.
fn assert_sound(predicted: &[Claim], actually: &[Ran]) -> Aside {
    let mut left = actually.to_vec();
    let mut aside = Aside::default();
    for (reached, ran) in predicted {
        if admitted_unknown(&ran.1) {
            aside.admitted += 1;
            continue;
        }
        // ⚠ The same join the report uses, not a rule of this test's own: the
        // fixture is asserted to exit 0, and exit 0 at the end of an `&&` chain
        // means every link in it ran. Only `Sometimes` survives that, which is
        // exactly the bucket the report never counts as certain either.
        if !reader::doing::Verdict::Ok.admits(*reached) {
            aside.uncertain += 1;
            continue;
        }
        match left.iter().position(|was| was == ran) {
            Some(at) => {
                left.remove(at);
            }
            None => panic!(
                "the reader claims a command the shell never ran:\n  claimed {ran:?}\n  \
                 the shell ran {actually:#?}"
            ),
        }
    }
    aside
}

/// And where the text determines everything, it claims all of them — with
/// nothing set aside, because there is nothing left undetermined to set aside.
fn assert_exact(predicted: &[Claim], actually: &[Ran]) {
    let aside = assert_sound(predicted, actually);
    assert_eq!(
        aside,
        Aside::default(),
        "the text determines this script, so nothing should have been left unresolved"
    );
    assert_eq!(
        predicted.len(),
        actually.len(),
        "the text determines this script, so the reader should have found every command\n  \
         predicted {predicted:#?}\n  the shell ran {actually:#?}"
    );
}

#[test]
fn a_directory_moved_by_cd_is_where_the_next_command_runs() {
    // ⚠ **The assumption everything else rests on, never once checked.** Every
    // relative path in the corpus is resolved against a directory this reader
    // decided — 48,172 `cd` operations' worth — and a wrong one does not show up
    // as a gap. It shows up as a confident, wrong path.
    let script = "mkdir -p src/geo && cd src && wc -l ../notes.md && cd geo && cat osm.ts";
    let (scratch, ran) = actually(
        script,
        &[
            ("notes.md", "one\ntwo\n"),
            ("src/geo/osm.ts", "export {}\n"),
        ],
    );
    assert_exact(&predicted(script, &scratch.0), &ran);
}

#[test]
fn a_loop_the_text_determines_runs_the_commands_the_reader_says_it_does() {
    // The reader runs this out statically; bash runs it for real. If unrolling
    // were wrong — an iteration too few, the wrong value bound — the two lists
    // would differ here and nowhere else in the suite.
    let script = "for f in a.log b.log c.log; do wc -l \"$f\"; done";
    let (scratch, ran) = actually(
        script,
        &[("a.log", "1\n"), ("b.log", "2\n"), ("c.log", "3\n")],
    );
    assert_exact(&predicted(script, &scratch.0), &ran);
}

#[test]
fn a_subshells_cd_does_not_escape_it() {
    // Unit-tested already, but from the reader's own model of a subshell. This
    // asks bash.
    let script = "mkdir -p sub && (cd sub && touch inside.txt) && touch outside.txt";
    let (scratch, ran) = actually(script, &[]);
    assert_exact(&predicted(script, &scratch.0), &ran);
}

#[test]
fn a_loop_counted_out_by_seq_runs_the_numbers_the_shell_ran() {
    // ⚠ **The largest class the reader still folded** — 1,029 loops in the
    // corpus, against 735 over a glob. `$(seq 1 4)` looks undetermined because it
    // carries a `$`, but every value is in the text: it is arithmetic, not a
    // question for the filesystem. bash and the reader should agree exactly, and
    // this is the one place that can say whether they do.
    let script = "for i in $(seq 1 4); do touch \"part-$i.txt\"; done";
    let (scratch, ran) = actually(script, &[]);
    assert_exact(&predicted(script, &scratch.0), &ran);
}

#[test]
fn seq_with_one_bound_starts_at_one_and_a_step_is_honoured() {
    // `seq N` is 1..=N and `seq FIRST STEP LAST` steps — getting either wrong
    // would put the reader's iterations out of step with the shell's silently.
    let script = "for i in $(seq 3); do touch a$i; done\n                  for j in $(seq 2 2 6); do touch b$j; done";
    let (scratch, ran) = actually(script, &[]);
    assert_exact(&predicted(script, &scratch.0), &ran);
}

#[test]
fn a_loop_over_a_glob_is_undercounted_and_never_invented() {
    // ⚠ **The property that lets the domain get richer.** The text does not
    // determine this list — the filesystem of the day did, and it is gone — so
    // the reader is *allowed* to miss the three `wc` calls. It is not allowed to
    // report one that did not happen. Every claim it does make must be here.
    //
    // When memview#819 lifts a glob to a bounded set, this is the test that says
    // the bound was honest, and it needs no old filesystem to say it.
    let script = "for f in *.log; do wc -l \"$f\"; done";
    let (scratch, ran) = actually(
        script,
        &[("a.log", "1\n"), ("b.log", "2\n"), ("c.log", "3\n")],
    );
    let predicted = predicted(script, &scratch.0);
    let aside = assert_sound(&predicted, &ran);
    assert_eq!(ran.len(), 3, "bash ran the body once per matching file");
    // One folded body, admitted as unresolved rather than claimed as an
    // execution. That is today's honest answer, and the number this test watches:
    // if the reader ever starts claiming three, they had better be the right
    // three, and this is where that would be caught.
    assert_eq!(
        aside,
        Aside {
            admitted: 1,
            uncertain: 0
        }
    );

    // ⚠ **`S ⊆ L`, checked against the shell rather than argued for.** The reader
    // says the loop's subject is *some subset of* `<root>/*.log`; bash says which
    // three files it actually was. Every one of them must match the pattern, or
    // the bound is a claim about an execution that did not happen — the one kind
    // of error a constrained unknown could introduce that a plain refusal could
    // not. Needs no old filesystem: the language is in the text.
    let cmds = reader::project::read(script).unwrap();
    let root = scratch.0.to_string_lossy().to_string();
    let found = reader::shell_files::extract(&cmds, Some(&root), HOME);
    let patterns: Vec<&String> = found.bounded.keys().collect();
    assert_eq!(patterns, [&format!("{root}/*.log")]);
    // A matcher that admitted everything would pass the loop below without
    // checking anything, which is the way a soundness test quietly dies.
    assert!(!matches(patterns[0], &format!("{root}/a.txt")));
    assert!(!matches(patterns[0], &format!("{root}/deep/a.log")));
    for (where_, argv) in &ran {
        // The shell was handed `a.log`, and the reader's bound is absolute
        // because a path only means anything against a directory. Same
        // resolution the reader does, from the cwd the shim recorded.
        let touched = argv.last().expect("wc was given a file");
        let touched = format!("{where_}/{touched}");
        assert!(
            matches(patterns[0], &touched),
            "the shell touched {touched}, which the reader's bound {} does not admit",
            patterns[0]
        );
    }
}

/// Does `path` match a shell pattern of `*` and `?`?
///
/// Written out rather than taken as a dependency: the whole point of the bound
/// is that membership is decidable, and three lines of it are worth less than a
/// crate on the argument that it might grow. Neither `*` nor `?` crosses `/`,
/// as in the shell.
fn matches(pattern: &str, path: &str) -> bool {
    fn go(p: &[u8], s: &[u8]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some(b'*') => {
                let mut at = 0;
                loop {
                    if go(&p[1..], &s[at..]) {
                        return true;
                    }
                    match s.get(at) {
                        Some(b'/') | None => return false,
                        Some(_) => at += 1,
                    }
                }
            }
            Some(b'?') => matches!(s.first(), Some(c) if *c != b'/') && go(&p[1..], &s[1..]),
            Some(c) => s.first() == Some(c) && go(&p[1..], &s[1..]),
        }
    }
    go(pattern.as_bytes(), path.as_bytes())
}

#[test]
fn only_one_arm_of_an_if_ran_and_the_reader_claims_neither_as_certain() {
    // ⚠ **The fabrication this harness was built to catch, and it caught it.**
    // Before memview#832 both arms came back [`Reached::Always`] — the label that
    // means "this definitely happened" — for two commands of which bash runs
    // exactly one. `assert_sound` fails on that: it would find `cat absent.txt`
    // claimed and no such line in the log.
    //
    // The condition is not a branch and stays certain: `grep` really does run.
    let script = "if grep -q one notes.md; then cat present.txt; else cat absent.txt; fi";
    let (scratch, ran) = actually(
        script,
        &[
            ("notes.md", "one\n"),
            ("present.txt", "taken\n"),
            ("absent.txt", "not taken\n"),
        ],
    );
    let predicted = predicted(script, &scratch.0);
    let aside = assert_sound(&predicted, &ran);
    assert_eq!(
        ran.len(),
        2,
        "bash ran the test and exactly one arm: {ran:#?}"
    );
    // The `grep` is confirmed against the log; both `cat`s are set aside. The
    // arm that *did* run is an undercount — the honest direction — and no
    // domain this size can do better, because the text alone cannot say which
    // way the test went.
    assert_eq!(
        aside,
        Aside {
            admitted: 0,
            uncertain: 2
        }
    );
}

#[test]
fn a_name_bound_to_a_literal_is_the_value_the_shell_used() {
    let script = "OUT=notes.md\nwc -l \"$OUT\"\ncat \"$OUT\"";
    let (scratch, ran) = actually(script, &[("notes.md", "one\n")]);
    assert_exact(&predicted(script, &scratch.0), &ran);
}

#[test]
fn a_nested_shell_runs_its_commands_where_the_wrapper_left_it() {
    // A third of the corpus goes through one of these, and the inner commands
    // are the ones that touch files. The wrapper has no counterpart in the log —
    // `bash` is not shimmed — so what is compared is exactly the leaves.
    let script = "mkdir -p pkg && cd pkg && bash -c 'touch built.js && wc -c built.js'";
    let (scratch, ran) = actually(script, &[]);
    assert_exact(&predicted(script, &scratch.0), &ran);
}

/// The real tool behind a shim, found before the shims exist.
fn which(verb: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(verb))
        .find(|at| at.is_file())
}

#[cfg(unix)]
fn make_runnable(at: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut mode = std::fs::metadata(at)
        .expect("the shim exists")
        .permissions();
    mode.set_mode(0o755);
    std::fs::set_permissions(at, mode).expect("the shim is runnable");
}
