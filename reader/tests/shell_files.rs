//! Which files a command used — against the commands the transcripts contain.
//!
//! Every case here is a shape taken from the corpus, and the negative ones carry
//! as much weight as the positive: this table's only real failure mode is
//! putting a path into an agent's record that the command never named.

use reader::project::read as parse;
use reader::shell_files::extract;

const HOME: &str = "/home/example";
const CWD: &str = "/home/example/Code/health";

/// Every file one script used, as `(path, wrote)` pairs.
fn uses(script: &str) -> Vec<(String, bool)> {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    extract(&cmds, Some(CWD), HOME)
        .files
        .into_iter()
        .map(|f| (f.path, f.write))
        .collect()
}

/// Everything one script's walk decided, for the cases that ask about more than
/// the files.
fn extracted(script: &str) -> reader::shell_files::Extract {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    extract(&cmds, Some(CWD), HOME)
}

/// The subjects the text did not determine, in the order they were met.
fn unnamed(script: &str) -> Vec<String> {
    let cmds = parse(script).unwrap();
    extract(&cmds, Some(CWD), HOME)
        .unnamed
        .into_iter()
        .flat_map(|(word, count)| std::iter::repeat_n(word, count))
        .collect()
}

/// The commands the table could not read, by name.
fn unread(script: &str) -> Vec<String> {
    let cmds = parse(script).unwrap();
    extract(&cmds, Some(CWD), HOME)
        .unhandled
        .into_keys()
        .collect()
}

#[test]
fn a_file_rewritten_through_a_temporary_is_a_write() {
    // The corpus's real shape for editing a file from the shell, and the reason
    // this exists at all: `Write` and `Edit` see none of it, so the agent doing
    // its editing this way loses the work.
    assert_eq!(
        uses("awk 'NR<1381' src/geo/velocity.ts > /tmp/new.ts; cp /tmp/new.ts src/geo/velocity.ts"),
        [
            ("/tmp/new.ts".to_string(), true),
            (
                "/home/example/Code/health/src/geo/velocity.ts".to_string(),
                false
            ),
            ("/tmp/new.ts".to_string(), false),
            (
                "/home/example/Code/health/src/geo/velocity.ts".to_string(),
                true
            ),
        ]
    );
}

#[test]
fn sed_writes_only_in_place() {
    // `-i` is the whole difference between reading a file and rewriting it, and
    // `-i.bak` and `-i''` are the same flag wearing a suffix.
    assert_eq!(
        uses("sed -n 325,350p src/geo/osm.ts"),
        [(
            "/home/example/Code/health/src/geo/osm.ts".to_string(),
            false
        )]
    );
    assert_eq!(
        uses("sed -i '' 's/a/b/' src/geo/osm.ts"),
        [("/home/example/Code/health/src/geo/osm.ts".to_string(), true)]
    );
}

#[test]
fn a_pattern_is_not_a_file() {
    // `grep pat f` opens the second word and not the first. Counting the pattern
    // would file work against a path named after whatever was searched for.
    assert_eq!(
        uses("grep -rn 'hsmmDecode' src/geo/velocity.ts"),
        [(
            "/home/example/Code/health/src/geo/velocity.ts".to_string(),
            false
        )]
    );
}

#[test]
fn a_flag_value_is_not_a_file_either() {
    // REGRESSION-shaped: without `-A` in grep's valued flags, `3` becomes the
    // pattern and `dhall` becomes a file — a word that is not a path, resolved
    // against the working directory and recorded as one.
    assert_eq!(
        uses("grep -A 3 dhall src/geo/osm.ts"),
        [(
            "/home/example/Code/health/src/geo/osm.ts".to_string(),
            false
        )]
    );
}

#[test]
fn cd_moves_the_directory_a_relative_path_resolves_against() {
    assert_eq!(
        uses("cd ~/Code/memview && cat src/store.rs"),
        [("/home/example/Code/memview/src/store.rs".to_string(), false)]
    );
}

#[test]
fn a_cd_inside_a_subshell_does_not_escape_it() {
    // `(cd x && …)` forks a shell, so the script it returns to has not moved.
    // Carrying the change outwards would resolve every later relative path
    // against the wrong directory — the invented-path failure, in bulk.
    assert_eq!(
        uses("(cd android && cat build.gradle.kts); cat Makefile"),
        [
            (
                "/home/example/Code/health/android/build.gradle.kts".to_string(),
                false
            ),
            ("/home/example/Code/health/Makefile".to_string(), false),
        ]
    );
}

#[test]
fn two_sibling_subshells_do_not_share_a_directory() {
    // A depth counter cannot tell these apart from one group containing both,
    // and the second `cat` would resolve under `frontend/`.
    assert_eq!(
        uses("(cd frontend && cat a.ts); (cd backend && cat b.rs)"),
        [
            ("/home/example/Code/health/frontend/a.ts".to_string(), false),
            ("/home/example/Code/health/backend/b.rs".to_string(), false),
        ]
    );
}

#[test]
fn an_unresolvable_cd_makes_the_directory_unknown_not_stale() {
    // Keeping the old directory would resolve `src/x.rs` somewhere the command
    // never ran. Nothing is the right answer.
    assert!(uses("cd \"$WORKDIR\" && cat src/x.rs").is_empty());
}

#[test]
fn an_unexpanded_variable_is_refused() {
    // There is no value to expand it to, and guessing one invents a file.
    assert!(uses("cat \"$OUT/report.txt\"").is_empty());
}

#[test]
fn a_subject_the_text_does_not_determine_is_counted_rather_than_dropped() {
    // ⚠ **The gap this closes (#92).** A refused word left no trace at all, so a
    // command that used a file nobody can name was recorded exactly like a
    // command that used none — and the second is a claim, not an absence. 592
    // distinct such subjects stood in the corpus, led by `$f`, `$p` and `$d`
    // inside loops whose list is a glob or a `$(…)`: the two things that genuinely
    // are not in the text, and so the two the reader must own up to rather than
    // quietly skip.
    //
    // Both refusals count, and they are refused in different places: a bare `$f`
    // never looks like a path at all, while `$d/report.txt` looks like one and
    // then resolves to nothing.
    //
    // ⚠ The glob case moved on 2026-08-13: a variable a glob loop bound is now
    // BOUNDED rather than merely unnamed, and lives in `Extract::bounded` — see
    // `a_glob_bounds_what_its_loop_variable_can_be`. It is still not a named
    // file, and `subjects_not_named` still counts it.
    assert_eq!(unnamed("for f in $(ls); do wc -l \"$f\"; done"), ["$f"]);
    assert_eq!(unnamed("cat \"$OUT/report.txt\""), ["$OUT/report.txt"]);
    // ⚠ **A backtick is an unmade expansion as much as a `$` is**, and testing
    // for `$` alone missed it — measured on the day the counter shipped, `cat
    // `which claude`` recorded no read and no admission of one. The older syntax
    // is rarer but it is not gone.
    //
    // ⚠ It is *shown* as `$( )` since the chain read through the tree
    // (`43ae9fe`), because the two spellings are one node there and the printer
    // picks one. What is admitted is the same admission; only the way it is
    // written down changed, and one spelling for one thing is the better answer
    // for a list a person reads.
    assert_eq!(unnamed("cat `which claude`"), ["$(which claude)"]);

    // Still no file, and that has not changed: naming what was refused must not
    // become a way of inventing it.
    assert!(uses("for f in *.log; do wc -l \"$f\"; done").is_empty());

    // A loop the text DOES determine is run out, so nothing is undetermined about
    // it — the words are values by the time the guard sees them.
    assert!(unnamed("for f in a.log b.log; do wc -l \"$f\"; done").is_empty());

    // And a word refused for any other reason is not a subject we failed to name.
    // `-` is stdin, a pattern is not a file, and `src` is a bare word the guard
    // throws away on purpose — counting those would turn an honest count into
    // noise nobody can act on.
    assert!(unnamed("rg pattern src").is_empty());
    assert!(unnamed("wc -l -").is_empty());
}

/// The patterns a script's subjects were bounded by, with their counts.
fn bounded(script: &str) -> Vec<String> {
    extracted(script)
        .bounded
        .into_iter()
        .flat_map(|(pattern, count)| std::iter::repeat_n(pattern, count))
        .collect()
}

#[test]
fn a_glob_bounds_what_its_loop_variable_can_be() {
    // ⚠ **A glob is not a shrug.** The directory it was answered against is gone,
    // so no file can be produced — but `some subset of src/*.ts` and `some file`
    // are different facts, and recording both as "not named" threw the first one
    // away. What the text says is `⟦*.log⟧ = some S ⊆ L(*.log) ∩ Files(dir, t)`:
    // an unknown finite subset of a KNOWN language.
    assert_eq!(
        bounded("for f in *.log; do wc -l \"$f\"; done"),
        ["/home/example/Code/health/*.log"]
    );
    // And it is off the unnamed list, because it is no longer that admission.
    assert!(unnamed("for f in *.log; do wc -l \"$f\"; done").is_empty());

    // Concatenation keeps the bound: a pattern followed by a literal is still a
    // pattern, which is the whole reason a regular language is the right ceiling.
    assert_eq!(
        bounded("for d in src/*; do cat \"$d/Cargo.toml\"; done"),
        ["/home/example/Code/health/src/*/Cargo.toml"]
    );

    // ⚠ **A transduction does not.** `${f%%:*}` is a rational function of the
    // variable, and honouring it needs the automaton this deliberately does not
    // build — so it stays opaque rather than being claimed as bounded.
    assert_eq!(
        unnamed("for f in *.log; do wc -l \"${f%%:*}\"; done"),
        ["${f%%:*}"]
    );
    assert!(bounded("for f in *.log; do wc -l \"${f%%:*}\"; done").is_empty());

    // A list whose own words are a question bounds nothing: which word the
    // variable took is then unknowable, and a bound that only sometimes holds is
    // worse than none.
    assert!(bounded("for f in $(ls); do wc -l \"$f\"; done").is_empty());
    assert_eq!(unnamed("for f in $(ls); do wc -l \"$f\"; done"), ["$f"]);

    // ⚠ A use inside a substitution sits one scope deeper than the loop that
    // bound the name, so the pattern has to be looked up through every enclosing
    // scope and not just this one. Worth 124 of the corpus's 269 bounded
    // subjects — it read as working on the simple case while missing a third of
    // them.
    assert_eq!(
        bounded("for f in *.log; do echo $(wc -l \"$f\"); done"),
        ["/home/example/Code/health/*.log"]
    );

    // Still no file. Bounding what was refused must not become a way of
    // inventing it.
    assert!(uses("for f in *.log; do wc -l \"$f\"; done").is_empty());

    // And a bounded subject is still NOT NAMED — a subset of a pattern is not a
    // file, and the headline must not quietly improve.
    assert_eq!(
        extracted("for f in *.log; do wc -l \"$f\"; done").subjects_not_named(),
        1
    );
}

#[test]
fn a_python_program_that_named_no_file_is_counted_with_the_shells_own() {
    // ⚠ **Two readers kept two accounts and only one was added up.** Python's
    // undetermined subjects outnumber the shell's in the corpus — 4,189 against
    // 3,007 — so a rate stated from `unnamed` alone read as though it covered
    // everything the reader does. `subjects_not_named` is the only thing that
    // adds them together, and it is derived rather than accumulated so a fourth
    // account cannot drift from the other three.
    let found = extracted("python3 -c 'import os; os.remove(f\"{d}/a.py\")'");
    // Nothing determined the path, so no file is claimed — and the miss is on
    // the record instead of vanishing.
    assert!(found.files.is_empty());
    assert!(
        found.unnamed.is_empty(),
        "this one is Python's, not the shell's"
    );
    assert_eq!(found.subjects_not_named(), 1);

    // A use whose path IS determined, refused by a rule of this layer rather
    // than by anything unknowable: `os.chdir` cannot be followed, so a relative
    // path out of that program names no file this reader can find.
    let moved = extracted("python3 -c 'import os; os.chdir(p); os.remove(\"a.py\")'");
    assert!(moved.files.is_empty());
    assert_eq!(moved.python.refused.moved, 1);
    assert_eq!(moved.subjects_not_named(), 1);

    // The identity the fold rests on: every use a program named is either kept
    // or refused, and the three refusal causes are the whole of the second.
    for script in [
        "python3 -c 'open(\"notes/out.md\", \"w\").write(x)'",
        "python3 -c 'import os; os.chdir(p); os.remove(\"a.py\")'",
        "python3 -c 'open(\"plain\", \"w\")'",
    ] {
        let found = extracted(script);
        assert_eq!(
            found.python.uses,
            found.python.kept + found.python.refused.total(),
            "{script}"
        );
    }
}

#[test]
fn home_is_the_one_expansion_with_a_knowable_value() {
    assert_eq!(
        uses("wc -l $HOME/Code/health/src/geo/osm.ts"),
        [(
            "/home/example/Code/health/src/geo/osm.ts".to_string(),
            false
        )]
    );
}

#[test]
fn a_seq_the_text_counts_out_is_run_like_any_other_list() {
    // ⚠ **The largest class the reader still folded (#821)** — 1,029 loops,
    // against 735 over a glob. `$(seq …)` carries a `$` and so failed the
    // determinacy test with every other substitution, but nothing about it is
    // unknown: it is arithmetic on numbers already in the text.
    //
    // The oracle checks these against bash itself; what is here is the arithmetic
    // and the refusals, which are cheaper to state than to run.
    assert_eq!(
        uses("for i in $(seq 1 3); do touch part-$i.txt; done"),
        [
            ("/home/example/Code/health/part-1.txt".to_string(), true),
            ("/home/example/Code/health/part-2.txt".to_string(), true),
            ("/home/example/Code/health/part-3.txt".to_string(), true),
        ]
    );
    // `seq N` counts from one; `seq FIRST STEP LAST` steps.
    assert_eq!(uses("for i in $(seq 2); do touch p$i.txt; done").len(), 2);
    assert_eq!(
        uses("for i in $(seq 1 3 9); do touch p$i.txt; done").len(),
        3
    );
    // ⚠ Counting down works and counting nowhere is an answer, not a failure:
    // `seq 3 1` prints nothing, so the body ran zero times — which is a fact, and
    // the reader should record no files rather than declining to read the loop.
    assert_eq!(
        uses("for i in $(seq 3 -1 1); do touch p$i.txt; done").len(),
        3
    );
    assert!(uses("for i in $(seq 3 1); do touch p$i.txt; done").is_empty());

    // A bound that is not in the text refuses the whole loop, which then folds
    // exactly as before — 6 such loops in the corpus, and what they printed is
    // genuinely gone.
    assert!(uses("for i in $(seq 1 $rounds); do touch p$i.txt; done").is_empty());
    assert_eq!(
        unnamed("for i in $(seq 1 $rounds); do touch p$i.txt; done"),
        ["p$i.txt"]
    );
    // And nothing else is run in the reader's head, however arithmetic it looks.
    assert!(uses("for i in $(ls); do touch p$i.txt; done").is_empty());
}

#[test]
fn another_machines_paths_stay_on_that_machine() {
    // `ssh` names no local file however its arguments look, and a `host:path`
    // operand is not a path here even though it parses as one.
    assert!(uses("ssh isis 'cat /etc/nixos/configuration.nix'").is_empty());
    assert_eq!(
        uses("scp isis:/var/log/fleet.log logs/fleet.log"),
        [("/home/example/Code/health/logs/fleet.log".to_string(), true)]
    );
}

#[test]
fn dev_null_is_plumbing_not_a_file() {
    // Left in, it is the busiest path in the entire corpus — 25,407 writes that
    // say nothing about anybody's work.
    assert!(uses("cargo test > /dev/null 2>&1").is_empty());
}

#[test]
fn a_redirect_counts_even_when_the_command_is_unknown() {
    // The command is unreadable; the file it wrote is not, and the two facts are
    // independent.
    assert_eq!(
        uses("./gradlew assembleDebug > build/out.log"),
        [
            ("/home/example/Code/health/build/out.log".to_string(), true),
            ("/home/example/Code/health/gradlew".to_string(), false),
        ]
    );
}

#[test]
fn a_loop_body_is_read_through_its_keyword() {
    // The grammar leaves `do` as an ordinary word, so the command behind it is
    // hidden until the keyword is stripped — 5,594 commands' worth.
    //
    // Twice, because the loop ran twice. A body that names the same file every
    // time still used it once per iteration, and the record says so.
    assert_eq!(
        uses("for f in a b; do cat src/geo/osm.ts; done"),
        [
            (
                "/home/example/Code/health/src/geo/osm.ts".to_string(),
                false
            ),
            (
                "/home/example/Code/health/src/geo/osm.ts".to_string(),
                false
            )
        ]
    );
}

#[test]
fn a_wrapper_is_not_the_command() {
    assert_eq!(
        uses("sudo rm -f /etc/hosts.bak"),
        [("/etc/hosts.bak".to_string(), true)]
    );
    assert_eq!(
        uses("REV=abc nohup bash scripts/verify.sh"),
        [(
            "/home/example/Code/health/scripts/verify.sh".to_string(),
            false
        )]
    );
}

#[test]
fn an_interpreters_script_is_a_file_and_its_code_is_not() {
    assert_eq!(
        uses("python3 scripts/report.py --days 7"),
        [(
            "/home/example/Code/health/scripts/report.py".to_string(),
            false
        )]
    );
    assert!(uses("python3 -c 'import os; print(os.getcwd())'").is_empty());
}

#[test]
fn git_reads_only_what_is_certainly_a_path() {
    // `git add` operands are paths; `git diff origin/main` is a revision, and
    // recording it would create a file called `origin/main`.
    // `git add` is NOT a change to the file: the edit already happened, and was
    // already counted where it happened. Counting the staging too was 37% of
    // every shell-derived write in the corpus — the same work, twice.
    assert!(uses("git -C ~/Code/memview add src/shell_files.rs").is_empty());
    // What remains does change the working tree.
    assert_eq!(
        uses("git -C ~/Code/memview rm src/gone.rs"),
        [("/home/example/Code/memview/src/gone.rs".to_string(), true)]
    );
    assert!(uses("git diff origin/main").is_empty());
    assert_eq!(
        uses("git log --oneline -- src/geo/osm.ts"),
        [(
            "/home/example/Code/health/src/geo/osm.ts".to_string(),
            false
        )]
    );
}

#[test]
fn a_bare_word_is_not_treated_as_a_path() {
    // The guard that keeps stray operands out, and its cost: `src` is a real
    // directory here and is lost with them. An undercount, by choice.
    assert!(uses("rg dhall src").is_empty());
}

#[test]
fn a_glob_is_recorded_as_written() {
    // Expanding it against today's checkout would attribute work to files that
    // did not exist then, and miss every file since deleted.
    assert_eq!(
        uses("cat plan/*.dhall"),
        [("/home/example/Code/health/plan/*.dhall".to_string(), false)]
    );
}

#[test]
fn an_unknown_command_contributes_nothing_and_is_counted() {
    // ⚠ The example was `ffmpeg`, then `dhall-to-json`, and both were taught
    // within the hour. The property under test belongs to whatever is still
    // unread; the command naming it is a placeholder by construction.
    assert!(uses("verified_cli --session 2026-06-21").is_empty());
    assert_eq!(
        unread("verified_cli --session 2026-06-21"),
        ["verified_cli"]
    );
}

#[test]
fn ffmpeg_puts_its_output_where_a_projection_can_see_it() {
    // 368 calls of real media — the recall pipeline's audio, and every one of
    // them was a file changed that nothing downstream could see.
    assert_eq!(
        uses("ffmpeg -hide_banner -i noisy.wav enhanced.wav"),
        [
            ("/home/example/Code/health/noisy.wav".to_string(), false),
            ("/home/example/Code/health/enhanced.wav".to_string(), true),
        ]
    );
}

#[test]
fn a_script_given_by_a_flag_leaves_no_operand_to_skip() {
    // REGRESSION. `sed 's/a/b/' f` and `sed -e 's/a/b/' f` take the same two
    // things in a different order; skipping a script operand that is not there
    // eats the file. It failed silently and in both directions — 19 `sed -i -e`
    // invocations in the corpus are writes that were recorded nowhere.
    assert_eq!(
        uses("sed -i '' -e 's/a/b/' -e 's/c/d/' src/geo/osm.ts"),
        [("/home/example/Code/health/src/geo/osm.ts".to_string(), true)]
    );
    assert_eq!(
        uses("grep -e 'hsmm' src/geo/osm.ts"),
        [(
            "/home/example/Code/health/src/geo/osm.ts".to_string(),
            false
        )]
    );
    // ...and the ordinary form still skips the script it really has.
    assert_eq!(
        uses("sed -i '' 's/a/b/' src/geo/osm.ts"),
        [("/home/example/Code/health/src/geo/osm.ts".to_string(), true)]
    );
}

#[test]
fn a_file_of_patterns_is_a_file_that_was_read() {
    // `-f` names a script or a pattern list. It is read whatever else happens,
    // and the operands after it are files rather than the pattern.
    assert_eq!(
        uses("grep -f scripts/patterns.txt src/geo/osm.ts"),
        [
            (
                "/home/example/Code/health/scripts/patterns.txt".to_string(),
                false
            ),
            (
                "/home/example/Code/health/src/geo/osm.ts".to_string(),
                false
            ),
        ]
    );
}

#[test]
fn a_local_shell_inside_a_command_is_read_too() {
    // A third of the corpus runs through a devshell wrapper — 15,366
    // `nix … -c`, 8,870 `nix-shell --run`, 6,974 `bash -c` — and every command
    // inside them was invisible while the string was left as one word.
    assert_eq!(
        uses("nix develop -c bash -c 'cat src/geo/osm.ts'"),
        [(
            "/home/example/Code/health/src/geo/osm.ts".to_string(),
            false
        )]
    );
    // Two layers and a wrapper: the real command is `biome`, and `--write`
    // makes it a write. This exact line appears 126 times in the corpus and
    // counted for nothing before.
    assert_eq!(
        uses("nix-shell -p nodejs_22 --run 'npx biome check --write src/geo/velocity.ts'"),
        [(
            "/home/example/Code/health/src/geo/velocity.ts".to_string(),
            true
        )]
    );
}

#[test]
fn the_test_runners_read_the_specs_they_are_given() {
    // ⚠ **The top of the unread list, and no grammar was needed for any of it.**
    // `vitest` 1,412 calls, `playwright` 1,330, `tsx` 1,459 — measured
    // 2026-08-06 — sitting unread behind the assumption that JavaScript meant a
    // parser. Their operands are spec files, which is a table row.
    assert_eq!(
        uses("vitest run src/geo/velocity.spec.ts"),
        [(
            "/home/example/Code/health/src/geo/velocity.spec.ts".to_string(),
            false
        )]
    );
    // A snapshot update is the one way either of them writes.
    assert_eq!(
        uses("vitest run -u src/geo/velocity.spec.ts"),
        [(
            "/home/example/Code/health/src/geo/velocity.spec.ts".to_string(),
            true
        )]
    );
    // A flag's value is not a subject: `--grep` takes a pattern.
    assert_eq!(
        uses("playwright test --grep smoke e2e/pages.spec.ts"),
        [(
            "/home/example/Code/health/e2e/pages.spec.ts".to_string(),
            false
        )]
    );
}

#[test]
fn a_name_bound_to_a_literal_is_the_path_it_holds() {
    // ⚠ **The largest unread name in the corpus, and its value is one line
    // above its use.** `$ADB` appears in 1,023 commands and **564 of them assign
    // it in the same command text**, usually to a literal nix-store path. It was
    // written off as unresolvable — measured 2026-08-06, it never was: the
    // reader simply had nowhere to keep a binding.
    assert_eq!(
        uses("ADB=/nix/store/abc-androidsdk/platform-tools/adb\ncat $ADB"),
        [(
            "/nix/store/abc-androidsdk/platform-tools/adb".to_string(),
            false
        )]
    );
    // `${NAME}` is written here too, and a name nobody bound is left as it was
    // — so the path guard refuses it rather than filing work against `$NOPE`.
    assert_eq!(
        uses("OUT=/tmp/report.json\ncat ${OUT} $NOPE"),
        [("/tmp/report.json".to_string(), false)]
    );
}

#[test]
fn a_name_bound_twice_is_trusted_no_further() {
    // Read top to bottom, "the last one wins" looks obvious. It is a guess the
    // moment a branch or a loop is involved, and this reader takes no branches —
    // the same line `python.rs` draws with its bind-exactly-once rule.
    assert!(uses("F=/tmp/a.txt\nF=/tmp/b.txt\ncat $F").is_empty());
    // The same value twice is not a rebinding, and the corpus does it constantly
    // — the same `ADB=…` in front of one command after another.
    assert_eq!(
        uses("F=/tmp/a.txt\nF=/tmp/a.txt\ncat $F"),
        [("/tmp/a.txt".to_string(), false)]
    );
}

#[test]
fn a_binding_inside_a_subshell_does_not_escape_it() {
    // The rule the working directory already follows, for the same reason: the
    // shell that held the binding is gone. Without this, `(F=x; …)` would resolve
    // every later `$F` in the script to a value that was never set there.
    assert!(uses("(F=/tmp/a.txt; cat $F)\ncat $F").len() == 1);
    // A prefix assignment binds for its own command and nothing after it.
    assert_eq!(
        uses("F=/tmp/a.txt cat $F\ncat $F"),
        [("/tmp/a.txt".to_string(), false)]
    );
}

#[test]
fn a_partly_known_value_names_no_file() {
    // A value carrying a `$` it cannot expand is kept, but only as far as it is
    // known. What must not happen is the suffix being taken for the whole path
    // — `/platform-tools/adb` is not where anything lives.
    assert!(uses("ADB=$ANDROID_HOME/platform-tools/adb\ncat $ADB").is_empty());
}

#[test]
fn a_partly_known_value_still_names_the_command() {
    // ⚠ **The unknown part of a value must not hide the known part.** 354 of
    // the corpus's assignments hold a `$` they cannot expand, and refusing them
    // whole threw away the one thing that was written down plainly: the name of
    // the tool being run. `$ADB` was the largest unread command in the corpus
    // at 1,071 calls — none of them a command called `$ADB`.
    // `adb` is a verb the table already knows, so those calls stop being unread
    // at all — the head was the only thing standing between them and it.
    assert!(unread("ADB=\"$ANDROID_HOME/platform-tools/adb\"\n$ADB shell ls").is_empty());
    // A tool the table does not know is now unread under its own name instead
    // of the variable's, which is what makes the unread list worth ranking.
    assert_eq!(unread("P=\"$ROOT/bin/probe\"\n$P --list"), ["probe"]);
    // And when it is a tool the table reads, its arguments come with it. The
    // head being unknown never said anything about the operands.
    assert_eq!(
        uses("PY=\"$VENV/bin/python\"\n$PY scripts/mine.py"),
        [(
            "/home/example/Code/health/scripts/mine.py".to_string(),
            false
        )]
    );
}

#[test]
fn a_value_that_must_be_run_binds_nothing() {
    // The one shape where keeping the text is worse than keeping nothing: the
    // substitution becomes the command's own name, and the index grows an entry
    // for a tool called `$(which adb)`. 13 of 1,023 `$ADB` uses, so the cost of
    // refusing is 13 commands and the cost of guessing is a corrupt index.
    assert_eq!(unread("ADB=$(which adb)\n$ADB shell ls"), ["$ADB"]);
    assert_eq!(unread("ADB=`which adb`\n$ADB shell ls"), ["$ADB"]);
}

#[test]
fn a_remote_shell_is_not_descended_into() {
    // **The boundary.** `ssh host '…'` is 6,068 calls whose paths belong to
    // another machine; reading them would file that machine's filesystem
    // against a local agent. Same for a container.
    assert!(uses("ssh isis 'cat /etc/nixos/configuration.nix'").is_empty());
    assert!(uses("ssh root@isis 'sed -i s/a/b/ /etc/hosts'").is_empty());
    assert!(uses("kubectl exec deploy/app -- sh -c 'cat /app/config.yaml'").is_empty());
    assert!(uses("docker exec api sh -c 'rm /srv/data.db'").is_empty());
}

#[test]
fn a_loop_over_a_literal_list_runs_once_per_value() {
    // ⚠ **The largest vanished subject in the corpus.** 6,474 shell `for` loops,
    // 4,524 of them over a literal word list — fully determined by the text and
    // read as nothing at all, because the loop variable expanded to nothing.
    // `$f` alone was refused 1,416 times, `$r` 338, `$d` 308.
    assert_eq!(
        uses("for f in a.txt b.txt; do cat $f; done"),
        [
            ("/home/example/Code/health/a.txt".to_string(), false),
            ("/home/example/Code/health/b.txt".to_string(), false),
        ]
    );
    // The corpus's real shape: a name per repository, joined to a path.
    assert_eq!(
        uses("for r in life coach; do cat $r/package.json; done"),
        [
            (
                "/home/example/Code/health/life/package.json".to_string(),
                false
            ),
            (
                "/home/example/Code/health/coach/package.json".to_string(),
                false
            ),
        ]
    );
    // A value binds inside the loop and nowhere after it. Bash would leave the
    // last one standing; refusing it is the safe direction, and inventing
    // `a.txt` for a command outside the loop is not.
    assert!(uses("for f in a.txt; do :; done\ncat $f").is_empty());
}

#[test]
fn a_loop_whose_list_is_not_determined_is_left_alone() {
    // A glob is answered by the filesystem of the day, which is gone; `$(…)`
    // is answered by running something. Neither is in the text, so neither is
    // unrolled — 727 glob loops and 330 substitutions stay dark on purpose.
    assert!(uses("for f in *.ts; do cat $f; done").is_empty());
    assert!(uses("for f in $(ls); do cat $f; done").is_empty());
    assert!(uses("for f in $LIST; do cat $f; done").is_empty());
}

#[test]
fn a_loop_inside_a_loop_is_unrolled_by_both() {
    // The inner loop is unrolled first, then duplicated by the outer — so the
    // body runs the product of the two lists, as it did.
    assert_eq!(
        uses("for d in x y; do for f in a.txt b.txt; do cat $d/$f; done; done"),
        [
            ("/home/example/Code/health/x/a.txt".to_string(), false),
            ("/home/example/Code/health/x/b.txt".to_string(), false),
            ("/home/example/Code/health/y/a.txt".to_string(), false),
            ("/home/example/Code/health/y/b.txt".to_string(), false),
        ]
    );
}

#[test]
fn a_cd_to_nowhere_knowable_leaves_no_directory_behind() {
    // ⚠ **The dangerous direction.** A `cd` whose destination cannot be read
    // was filed as an unknown command, which left the *previous* directory in
    // force — so every relative path after it resolved against a directory the
    // script had already left. That is this table's one unacceptable failure:
    // a path in an agent's record that no command ever named.
    assert!(uses("cd $BUILD\ncat config.json").is_empty());
    // Where it went is unknown; that it went somewhere is not. The domain
    // already holds "a directory nobody can name", so this is a change of
    // directory rather than an unreadable command.
    assert!(unread("cd $BUILD\ncat config.json").is_empty());
    // It poisons its own scope and no other, exactly as a successful `cd` moves
    // only its own — the script the subshell returns to is untouched.
    assert_eq!(
        uses("(cd $BUILD; cat a.txt)\ncat b.txt"),
        [("/home/example/Code/health/b.txt".to_string(), false)]
    );
}

#[test]
fn a_cd_inside_a_nested_shell_does_not_escape_it() {
    // `bash -c 'cd x && …'` moves that shell alone, exactly as a subshell does.
    assert_eq!(
        uses("bash -c 'cd frontend && cat a.ts'; cat b.rs"),
        [
            ("/home/example/Code/health/frontend/a.ts".to_string(), false),
            ("/home/example/Code/health/b.rs".to_string(), false),
        ]
    );
}

#[test]
fn only_a_shells_inline_flag_is_shell() {
    // `python -c` carries Python and `node -e` carries JavaScript. Read as
    // *shell* either would invent commands nobody ran. Each is read by the
    // reader that knows it, and resolved here — where the working directory is.
    assert_eq!(
        uses("python3 -c 'import os; os.remove(\"src/a.py\")'"),
        [("/home/example/Code/health/src/a.py".to_string(), true)]
    );
    // ⚠ **This asserted `is_empty()` until 2026-08-22**, when `node -e` was
    // ranked as a query tool on a count of its writes alone. Its READS are
    // 1,790 `readFileSync` calls across the corpus, and a projection is mostly
    // about reads — so the decision moved when the denominator did.
    assert_eq!(
        uses("node -e 'require(\"fs\").readFileSync(\"src/a.ts\")'"),
        [("/home/example/Code/health/src/a.ts".to_string(), false)]
    );
}

#[test]
fn a_command_run_from_inside_a_carried_program_is_followed_home() {
    // The loop the three readers close between them: a shell that runs Python
    // that runs a shell. Until this, everything past the `os.system` was
    // invisible — and `subprocess.run` alone was 443 calls of it.
    assert_eq!(
        uses("python3 -c 'import os; os.system(\"cat src/a.py\")'"),
        [("/home/example/Code/health/src/a.py".to_string(), false)]
    );
    // And a JavaScript one, whose `execSync` really does go through a shell.
    assert_eq!(
        uses("node -e 'require(\"child_process\").execSync(\"cat src/a.ts\")'"),
        [("/home/example/Code/health/src/a.ts".to_string(), false)]
    );
    // ⚠ The `cd` inside stays inside, exactly as it does for `bash -c`.
    assert_eq!(
        uses("python3 -c 'import os; os.system(\"cd frontend && cat a.ts\")'"),
        [("/home/example/Code/health/frontend/a.ts".to_string(), false)]
    );
}

#[test]
fn another_machines_script_is_read_but_never_filed_here() {
    // Refusing to *parse* it was the cruder version of the rule: it kept the
    // local index clean by knowing nothing at all about 6,068 calls. What those
    // scripts do to `/etc/nixos` is real knowledge — it just belongs to the host.
    let cmds = parse("ssh root@isis.xinutec.org 'cat /etc/nixos/configuration.nix'").unwrap();
    let found = extract(&cmds, Some(CWD), HOME);
    assert!(found.files.is_empty(), "nothing local: {:?}", found.files);
    assert_eq!(found.remote.len(), 1);
    assert_eq!(found.remote[0].host, "isis");
    assert_eq!(found.remote[0].path, "/etc/nixos/configuration.nix");
    assert!(!found.remote[0].write);
}

#[test]
fn a_remote_script_has_no_local_working_directory() {
    // This machine's directory means nothing there, so a relative path is
    // unusable — unless the script goes somewhere first, which many do.
    let cmds = parse("ssh odin 'sed -i s/a/b/ hosts'").unwrap();
    assert!(extract(&cmds, Some(CWD), HOME).remote.is_empty());

    let cmds = parse("ssh odin 'cd /etc/nixos && sed -i s/a/b/ flake.nix'").unwrap();
    let found = extract(&cmds, Some(CWD), HOME);
    assert_eq!(found.remote.len(), 1);
    assert_eq!(found.remote[0].path, "/etc/nixos/flake.nix");
    assert!(found.remote[0].write);
    assert!(found.files.is_empty());
}

#[test]
fn one_machine_under_its_several_names() {
    // The corpus writes `root@isis`, `root@isis.xinutec.org` and `isis` for the
    // same host — and an IP is a name in its own right, not a name with a
    // domain on it: the first label of 192.168.1.133 is a host called `192`,
    // which three machines would share.
    let host = |cmd: &str| {
        let cmds = parse(cmd).unwrap();
        extract(&cmds, Some(CWD), HOME).remote[0].host.clone()
    };
    assert_eq!(host("ssh root@isis.xinutec.org 'cat /etc/hosts'"), "isis");
    assert_eq!(host("ssh isis 'cat /etc/hosts'"), "isis");
    assert_eq!(host("ssh -p 2222 pippijn@isis 'cat /etc/hosts'"), "isis");
    assert_eq!(host("ssh 192.168.1.133 'cat /etc/hosts'"), "192.168.1.133");
}

#[test]
fn a_host_named_by_a_variable_is_not_a_host() {
    // `ssh "$h" '…'` cannot be resolved to a machine, and a use filed against
    // `$h` is filed against nothing.
    let cmds = parse("ssh \"$h\" 'cat /etc/hosts'").unwrap();
    let found = extract(&cmds, Some(CWD), HOME);
    assert!(found.remote.is_empty());
    assert!(found.files.is_empty());
}

#[test]
fn a_container_is_another_machine_too() {
    let cmds = parse("kubectl -n home exec deploy/home-db -- sh -c 'cat /etc/my.cnf'").unwrap();
    let found = extract(&cmds, Some(CWD), HOME);
    assert!(found.files.is_empty());
    assert_eq!(found.remote.len(), 1);
    assert_eq!(found.remote[0].host, "deploy/home-db");
    assert_eq!(found.remote[0].path, "/etc/my.cnf");
}

#[test]
fn a_python_heredoc_changes_files_like_any_other_command() {
    // The shape this whole reader exists for, and one no `Write`, `Edit` or
    // `sed` records: 3,547 writes in the corpus are inside a body like this.
    assert_eq!(
        uses(
            "python3 - <<'PY'\nfrom pathlib import Path\np = Path('src/geo/velocity.ts')\np.write_text(p.read_text().upper())\nPY"
        ),
        [
            // The read happens first: it is the argument the write is given.
            (
                "/home/example/Code/health/src/geo/velocity.ts".to_string(),
                false
            ),
            (
                "/home/example/Code/health/src/geo/velocity.ts".to_string(),
                true
            ),
        ]
    );
}

#[test]
fn python_inside_a_devshell_wrapper_is_still_python() {
    // A third of the corpus runs through one of these, and the heredoc body has
    // to survive being quoted, re-parsed and classified again to get here.
    assert_eq!(
        uses("nix develop -c bash -c 'python3 - <<PY\nopen(\"notes/out.md\", \"w\").write(x)\nPY'"),
        [("/home/example/Code/health/notes/out.md".to_string(), true)]
    );
}

#[test]
fn a_heredoc_that_is_not_python_is_not_read_as_python() {
    // `cat > f <<EOF` is already a write, recorded through the redirect. Reading
    // the body as a program as well would count the same work twice and would
    // read prose as code.
    assert_eq!(
        uses("cat > notes/plan.md <<'EOF'\nopen('src/other.ts', 'w')\nEOF"),
        [("/home/example/Code/health/notes/plan.md".to_string(), true)]
    );
}

#[test]
fn python_run_on_another_machine_stays_on_that_machine() {
    let cmds =
        parse("ssh root@isis.xinutec.org 'python3 - <<PY\nopen(\"/etc/nixos/x.nix\", \"w\")\nPY'")
            .unwrap();
    let found = extract(&cmds, Some(CWD), HOME);
    assert!(found.files.is_empty(), "no local file was touched");
    assert_eq!(found.remote.len(), 1);
    assert_eq!(found.remote[0].host, "isis");
    assert_eq!(found.remote[0].path, "/etc/nixos/x.nix");
    assert!(found.remote[0].write);
}

#[test]
fn a_program_that_moves_its_own_directory_keeps_only_anchored_paths() {
    // `os.chdir` cannot be followed, so a relative path after one is a guess —
    // and a wrong directory is how a real path becomes an invented one.
    assert_eq!(
        uses(
            "python3 - <<'PY'\nimport os\nos.chdir('/tmp/build')\nopen('relative.txt', 'w')\nopen('/tmp/build/absolute.txt', 'w')\nPY"
        ),
        [("/tmp/build/absolute.txt".to_string(), true)]
    );
}

#[test]
fn a_cd_into_the_directory_it_is_already_in_moves_nothing() {
    // Measured against the corpus's 90 such calls: the doubling never happened.
    // In 33 the `cd` failed outright — the session had moved there earlier and
    // the agent said so again — and in the other 57 the command succeeded,
    // which means the recorded directory was already the one being entered.
    // Applying the move is wrong either way, and refusing it is right either way.
    let cmds = parse("cd health && sed -n '1,5p' src/geo/osm.ts").unwrap();
    assert_eq!(
        extract(&cmds, Some(CWD), HOME)
            .files
            .into_iter()
            .map(|f| f.path)
            .collect::<Vec<_>>(),
        ["/home/example/Code/health/src/geo/osm.ts"]
    );
    // A directory that really is one level down still moves, doubled name or not.
    assert_eq!(
        uses("cd frontend && cat main.ts"),
        [(
            "/home/example/Code/health/frontend/main.ts".to_string(),
            false
        )]
    );
}

/// Every file use, as `(path, wrote, what had to hold)`.
fn conditions(script: &str) -> Vec<(String, bool, reader::shell::Reached)> {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    extract(&cmds, Some(CWD), HOME)
        .files
        .into_iter()
        .map(|f| (f.path, f.write, f.reached))
        .collect()
}

#[test]
fn only_the_last_turn_of_a_loop_ends_in_the_reported_status() {
    // ⚠ **A loop reports one status: its last iteration's.** So an `&&` in the
    // body is confirmable for that turn alone and unconfirmable for every
    // earlier one — and that is visible only once the body has been run out into
    // one copy per value, which happens after the parser has had its say.
    use reader::shell::Reached;
    let found = conditions("for f in a.txt b.txt c.txt; do cat $f && wc -l $f; done");
    let conds: Vec<reader::shell::Reached> = found.iter().map(|(_, _, r)| *r).collect();
    assert_eq!(
        conds,
        [
            // Each `cat` opens the loop body and runs every time round.
            Reached::Always,
            Reached::Sometimes,
            Reached::Always,
            Reached::Sometimes,
            Reached::Always,
            // Only this one's success is what the call reported.
            Reached::OnSuccess,
        ]
    );
}

// ---------------------------------------------------------------------------
// The trace: the same walk, saying what it did as it did it.
//
// These exist because a *view* of the parse is only worth showing if it is the
// walk's own account. Every case below is therefore written as an agreement
// between `trace` and `extract` on the same script, rather than as a fact about
// the trace alone: the moment the two can differ, the view starts lying about
// what the index was built from.
// ---------------------------------------------------------------------------

use reader::shell_files::trace;

/// Each step's words and what had to hold for it to run.
fn reached(script: &str) -> Vec<(String, reader::shell::Reached)> {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    trace(&cmds, Some(CWD), HOME)
        .steps
        .into_iter()
        .map(|s| (s.argv.join(" "), s.reached))
        .collect()
}

#[test]
fn a_loop_body_that_may_never_have_run_is_not_certain() {
    use reader::shell::Reached::{Always, Sometimes};

    // ⚠ **`while` tests before the first iteration**, so an empty input runs the
    // body no times — and recording it as certainly run is the same over-claim
    // as claiming both arms of an `if`. 4,544 of the corpus's calls carry a
    // `while` or an `until`.
    assert_eq!(
        reached("while read l; do cat x.ts; done < in.txt"),
        [
            ("read l".to_string(), Always),
            ("cat x.ts".to_string(), Sometimes),
            (String::new(), Always), // `done < in.txt`, the redirect's own step
        ]
    );

    // ⚠ **But a glob loop DID run its body**, and demoting every folded loop
    // would trade one over-claim for a bigger under-claim. With `nullglob` off —
    // the default, and what these calls ran under — a pattern matching nothing
    // expands to *itself*, so the body runs once with the pattern as the value.
    assert_eq!(
        reached("for f in *.log; do wc -l \"$f\"; done"),
        [
            ("for f in *.log".to_string(), Always),
            ("wc -l $f".to_string(), Always),
        ]
    );

    // A list that is a question can be empty, and then the body never ran.
    assert_eq!(
        reached("for f in $(ls); do wc -l \"$f\"; done"),
        [
            ("ls".to_string(), Always),
            ("for f in $(ls)".to_string(), Always),
            ("wc -l $f".to_string(), Sometimes),
        ]
    );

    // ⚠ A determinate loop INSIDE an uncertain one is still run out — the values
    // are in the text — but every iteration inherits the doubt. Getting this
    // wrong is why `closing_done` had to stop using `unwrap_command`, which
    // strips `while` and so never counted one: a `while` loop's `done` closed
    // nothing at all, and no rule that works on loop spans could see the body.
    assert_eq!(
        reached("while read l; do for f in a.ts b.ts; do cat $f; done; done < in.txt"),
        [
            ("read l".to_string(), Always),
            ("for f in a.ts b.ts".to_string(), Sometimes),
            ("cat a.ts".to_string(), Sometimes),
            ("cat b.ts".to_string(), Sometimes),
            (String::new(), Always),
        ]
    );
}

/// The steps of one script, as `(depth, first word, how many uses)`.
fn walked(script: &str) -> Vec<(usize, String, usize)> {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    trace(&cmds, Some(CWD), HOME)
        .steps
        .into_iter()
        .map(|s| {
            (
                s.depth,
                s.argv.first().cloned().unwrap_or_default(),
                s.files.len() + s.away.len(),
            )
        })
        .collect()
}

#[test]
fn tracing_changes_nothing_about_what_was_found() {
    // The point of the flag: it adds an account of the walk and moves no result
    // of it. Asserted on the shape that exercises every layer at once — a
    // devshell wrapper, a redirect on it, a `cd` inside, and a use after.
    let script = "nix develop -c bash -c 'cd frontend && cp a.ts b.ts' > /tmp/log 2>&1";
    let cmds = parse(script).unwrap();
    let plain = extract(&cmds, Some(CWD), HOME);
    let traced = trace(&cmds, Some(CWD), HOME);
    assert_eq!(plain.files, traced.files);
    assert_eq!(plain.remote, traced.remote);
    assert_eq!(plain.handled, traced.handled);
    assert_eq!(plain.unhandled, traced.unhandled);
    assert!(plain.steps.is_empty(), "extract must not pay for the trace");
    assert!(!traced.steps.is_empty());
}

#[test]
fn every_use_belongs_to_exactly_one_step() {
    // ⚠ **The defect this guards is double-counting at two depths.** A wrapper
    // absorbs the files of the script it opens; if its own step claimed them
    // too, the view would show one `cp` as two writes — the bug being that the
    // totals would still be right, so nothing else would ever catch it.
    let script = "nix develop -c bash -c 'cp a.ts b.ts' > /tmp/log";
    let cmds = parse(script).unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    let claimed: usize = traced.steps.iter().map(|s| s.files.len()).sum();
    assert_eq!(claimed, traced.files.len());
}

#[test]
fn a_wrapper_stands_in_front_of_what_it_opens() {
    // Outwards-in, the order the shell opens them — so indenting by `depth`
    // draws the nesting without the reader having to sort anything.
    //
    // ⚠ **`nix develop -c bash -c '…'` is ONE step, not two.** `-c` on `nix`
    // carries an argv rather than a script, so the wrapper is unwrapped and the
    // command that gets classified is the `bash -c` inside it — which is where
    // the one and only parse of a nested script happens. Two visible wrappers,
    // one nesting: the view would be wrong to draw two indents here, and this is
    // the case that says so.
    assert_eq!(
        walked("nix develop -c bash -c 'cd frontend && cat main.ts'"),
        [
            // The wrapper named no file itself; its script is followed, not read.
            (0, "nix".to_string(), 0),
            (1, "cd".to_string(), 0),
            (1, "cat".to_string(), 1),
        ]
    );
}

#[test]
fn a_wrappers_own_redirect_stays_with_the_wrapper() {
    // The one thing a wrapper *does* name. It must not fall through to the
    // inner command, which never saw it.
    let cmds = parse("nix develop -c bash -c 'cp a.ts b.ts' > /tmp/log").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    let wrapper = &traced.steps[0];
    assert_eq!(wrapper.argv.first().unwrap(), "nix");
    assert_eq!(
        wrapper.files,
        [reader::shell_files::FileUse {
            path: "/tmp/log".to_string(),
            write: true,
            reached: reader::shell::Reached::Always,
        }]
    );
}

#[test]
fn a_step_says_which_directory_it_resolved_against() {
    // The question the view exists to answer — why *that* path — and for a
    // relative operand the answer is always the directory, which the text does
    // not show.
    let cmds = parse("cd frontend && cat main.ts").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    let read = traced.steps.last().unwrap();
    assert_eq!(read.argv, ["cat", "main.ts"]);
    assert_eq!(
        read.cwd.as_deref(),
        Some("/home/example/Code/health/frontend")
    );
    assert_eq!(
        read.files[0].path,
        "/home/example/Code/health/frontend/main.ts"
    );
}

#[test]
fn a_step_carries_the_words_the_shell_would_have_run() {
    // Not the words as written. A loop variable and an assignment are both
    // gone by the time a path is resolved, and a reader looking at `$f` cannot
    // see why the file came out as it did.
    //
    // ⚠ **The keyword that introduced a command is not part of it.** `do`/`then`
    // are ordinary words to the grammar by design — a keyword rule would have to
    // decide whether `echo done` ends a loop — so they arrive on the front of
    // the body's words and are taken off here, where a step is the one thing
    // that gets shown. `done` is not a command at all and gets no step.
    //
    // The `for` head stays whole, and that is the deliberate exception: it is
    // not a command either, but it carries the list the loop ran over, which for
    // a folded glob is the only place the pattern appears. `f in a.ts b.ts` with
    // the `for` removed would not be anything.
    assert_eq!(
        walked("for f in a.ts b.ts; do cat $f; done")
            .into_iter()
            .map(|(_, word, _)| word)
            .collect::<Vec<_>>(),
        ["for", "cat", "cat"]
    );
    let cmds = parse("for f in a.ts b.ts; do cat $f; done").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    // The body once per value, with `$f` gone — which is the actual claim.
    assert_eq!(traced.steps[1].argv, ["cat", "a.ts"]);
    assert_eq!(traced.steps[2].argv, ["cat", "b.ts"]);

    // A branch keyword goes the same way, and the condition keeps its command.
    let cmds = parse("if grep -q x a.txt; then cat b.txt; fi").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    assert_eq!(traced.steps[0].argv, ["grep", "-q", "x", "a.txt"]);
    assert_eq!(traced.steps[1].argv, ["cat", "b.txt"]);
    assert_eq!(traced.steps.len(), 2, "`fi` is not a command");

    // ⚠ A wrapper is the opposite answer to the same question: `sudo` ran, and
    // hiding it would misdescribe how the file was changed.
    let cmds = parse("sudo rm -rf /tmp/x").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    assert_eq!(traced.steps[0].argv, ["sudo", "rm", "-rf", "/tmp/x"]);
}

#[test]
fn a_closing_word_that_redirects_still_gets_its_step() {
    // `done < in.txt` feeds the whole loop from a file. The word is not a
    // command and would take no step, but the read is real — and a use that
    // belongs to no step is the one thing `every_use_belongs_to_exactly_one_step`
    // forbids.
    let cmds = parse("while read l; do echo $l; done < in.txt").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    let redirect = traced.steps.last().unwrap();
    assert!(redirect.argv.is_empty());
    assert_eq!(
        redirect
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        ["/home/example/Code/health/in.txt"]
    );
}

#[test]
fn a_line_that_is_only_a_redirect_still_gets_a_step() {
    // 8,386 of the corpus's commands are a redirect and nothing else. Without a
    // step they would write a file that the view never shows — the one failure
    // mode that makes a debugging view actively misleading.
    let cmds = parse("> /tmp/log").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    assert_eq!(traced.steps.len(), 1);
    assert!(traced.steps[0].argv.is_empty());
    assert!(
        traced.steps[0].op.is_none(),
        "no command was classified, so there is no operation to name"
    );
    assert_eq!(traced.steps[0].files.len(), 1);
    assert!(traced.steps[0].files[0].write);
}

#[test]
fn a_remote_commands_files_never_reach_the_local_ones() {
    // The separation `RemoteUse` exists for, held at the step level too: a view
    // that showed isis's paths beside this machine's would undo it by eye.
    let cmds = parse("ssh root@isis.xinutec.org 'cat /etc/nixos/configuration.nix'").unwrap();
    let traced = trace(&cmds, Some(CWD), HOME);
    let inner = traced.steps.last().unwrap();
    assert_eq!(inner.host.as_deref(), Some("isis"));
    assert!(inner.files.is_empty(), "nothing local was used");
    assert_eq!(inner.away.len(), 1);
    assert_eq!(inner.away[0].path, "/etc/nixos/configuration.nix");
}

#[test]
fn a_download_saved_by_flag_is_a_write() {
    // ⚠ **27% of the corpus's curl/wget calls, writing files credited to
    // nobody.** `curl URL > file` was always counted, because a redirect is
    // collected whatever the command is; `curl -o file URL` was not — the same
    // shape as the `sed -e` defect, where an operand given by a flag leaves
    // nothing in the operand position to notice.
    assert_eq!(
        uses("curl -sS -o /tmp/state.json https://home.xinutec.org/api/usage"),
        [("/tmp/state.json".to_string(), true)]
    );
    assert_eq!(
        uses("wget -O logs/fleet.log https://example.org/fleet.log"),
        [("/home/example/Code/health/logs/fleet.log".to_string(), true)]
    );
}

#[test]
fn a_download_that_names_no_local_file_still_names_none() {
    // The reason these sat in the no-files list in the first place, and it is
    // still right for every other shape: a URL is not a path, and what is at the
    // far end is not this machine's.
    assert!(uses("curl -sS https://home.xinutec.org/api/usage").is_empty());
    assert!(
        uses("curl -X POST -H 'content-type: application/json' -d '{}' http://127.0.0.1:8096/api")
            .is_empty()
    );
}

#[test]
fn curls_capital_o_names_a_file_only_wget_can_resolve() {
    // ⚠ **The same letter, opposite meanings.** wget's `-O` takes the name;
    // curl's takes none and derives it from the URL's last segment — which this
    // reader cannot resolve without the URL, and must therefore refuse rather
    // than guess. Guessing here would invent a path, which is the one failure
    // that makes every count downstream a lie.
    assert!(uses("curl -O https://example.org/dist/app.tar.gz").is_empty());
    assert_eq!(
        uses("wget -O /tmp/app.tar.gz https://example.org/dist/app.tar.gz"),
        [("/tmp/app.tar.gz".to_string(), true)]
    );
}

#[test]
fn a_download_to_a_redirect_is_still_counted_once() {
    // The spelling that always worked. Both forms must now agree, and neither
    // may double-count.
    assert_eq!(
        uses("curl -sS https://example.org/x.json > /tmp/x.json"),
        [("/tmp/x.json".to_string(), true)]
    );
}

/// Every file one script used, told which `cd` targets the shell refused.
fn uses_knowing(script: &str, refused: &[&str]) -> Vec<(String, bool)> {
    let cmds = parse(script).unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"));
    let refused: Vec<String> = refused.iter().map(|it| (*it).to_string()).collect();
    reader::shell_files::extract_knowing(&cmds, Some(CWD), HOME, &refused)
        .files
        .into_iter()
        .map(|f| (f.path, f.write))
        .collect()
}

#[test]
fn a_cd_the_shell_refused_does_not_move_the_directory() {
    // Reported from the console 2026-08-08 by typing it: `cd memcheck` into a
    // directory with no `memcheck` in it, then reading a file. The `cat` ran
    // where it already was, and the path recorded was under a directory that has
    // never existed.
    let script = "cd nowhere; cat Cargo.toml";
    assert_eq!(
        uses(script),
        vec![(
            "/home/example/Code/health/nowhere/Cargo.toml".to_string(),
            false
        )],
        "with nothing known the move has to be taken at its word",
    );
    assert_eq!(
        uses_knowing(script, &["nowhere"]),
        vec![("/home/example/Code/health/Cargo.toml".to_string(), false)],
        "the shell said it never entered `nowhere`",
    );
}

#[test]
fn only_the_refused_cd_is_held_back() {
    // A script that changes directory twice, one of which worked. Holding both
    // back would be the same defect pointing the other way.
    let uses = uses_knowing("cd gone; cd lean && cat lakefile.toml", &["gone"]);
    assert_eq!(
        uses,
        vec![(
            "/home/example/Code/health/lean/lakefile.toml".to_string(),
            false
        )],
    );
}

#[test]
fn a_refusal_is_matched_on_the_word_not_the_path() {
    // The shell echoes the operand as written, so a trailing slash on either
    // side is the same target.
    assert_eq!(
        uses_knowing("cd nowhere/; cat Cargo.toml", &["nowhere"]),
        vec![("/home/example/Code/health/Cargo.toml".to_string(), false)],
    );
}

#[test]
fn a_refusal_from_one_command_does_not_silence_another() {
    // `nowhere` was refused; `lean` was not, and its `cd` must still apply even
    // though a refusal was reported somewhere in the same call.
    assert_eq!(
        uses_knowing("cd lean; cat lakefile.toml", &["nowhere"]),
        vec![(
            "/home/example/Code/health/lean/lakefile.toml".to_string(),
            false
        )],
    );
}

#[test]
fn a_cd_into_the_directory_the_line_is_already_in_moves_nothing() {
    // ⚠ **The transcript's `cwd` means two different things** — measured
    // 2026-08-12 across 191,273 `Bash` calls: 168 single-call lines record the
    // directory their command STARTED in and 84 record the one it ENDED in, in
    // the same transcript at the same CLI version (memview #449). On a line of
    // the second kind, applying the command's own `cd` again doubles the
    // segment: all 84 landed in a directory that has never existed.
    //
    // Under the first reading the same shape is a `cd` the shell refused, which
    // also moves nothing. So the rule needs no verdict on which reading is right.
    assert_eq!(
        uses("cd health && cat Cargo.toml"),
        [("/home/example/Code/health/Cargo.toml".to_string(), false)]
    );
}

#[test]
fn a_multi_segment_cd_the_line_already_ends_with_moves_nothing_either() {
    // `cd frontend/src/app` recorded at `…/frontend/src/app` — the commonest
    // shape of the 84, and the one that produced
    // `…/frontend/src/app/frontend/src/app`.
    let cmds = parse("cd frontend/src/app && cat main.ts").unwrap();
    let files: Vec<String> = extract(
        &cmds,
        Some("/home/example/Code/health/frontend/src/app"),
        HOME,
    )
    .files
    .into_iter()
    .map(|f| f.path)
    .collect();
    assert_eq!(
        files,
        ["/home/example/Code/health/frontend/src/app/main.ts"]
    );
}

#[test]
fn a_cd_into_a_directory_of_the_same_name_deeper_down_still_moves() {
    // The rule is a TAIL match, not a name match: `cd src` from `…/health` is an
    // ordinary move and must apply, even though a `src` exists further down.
    assert_eq!(
        uses("cd src && cat main.rs"),
        [("/home/example/Code/health/src/main.rs".to_string(), false)]
    );
}

#[test]
fn an_installed_program_run_by_its_path_is_not_a_file_that_was_read() {
    // ⚠ **The reader's one forbidden error, in miniature.** `/bin/sleep 5` used
    // no file at all, and recording the binary put it 135th busiest in the
    // corpus — a path nobody read, in an index whose whole purpose is the files
    // an agent worked on. Same for the interpreter of `python -c`, and for adb,
    // which the Android SDK reaches through a `libexec` component.
    assert!(uses("/bin/sleep 5").is_empty());
    assert!(uses("/home/example/.venv/bin/python -c 'print(1)'").is_empty());
    assert!(
        uses("/nix/store/abc-androidsdk/libexec/android-sdk/platform-tools/adb devices").is_empty()
    );
    assert!(uses("/home/example/Code/lares/rust/target/release/lares --once").is_empty());
}

#[test]
fn a_script_in_the_work_is_still_recorded_when_it_is_run() {
    // ⚠ **The half that must NOT move, and the reason the test is the path
    // rather than the verb.** #799 proposed "the basename resolves in the verb
    // table, so it is a program" — and `gradlew` IS in that table, beside `mvn`,
    // `pip` and `ng`. Measured by ablation over 73,907 Bash calls: that rule
    // deleted 2,110 reads of `./gradlew` across the fleet's Android repos to
    // remove ~800 of the noise it was aimed at. Running a script somebody wrote
    // is a use of that script.
    assert_eq!(
        uses("./gradlew assembleDebug"),
        [("/home/example/Code/health/gradlew".to_string(), false)]
    );
    assert_eq!(
        uses("/home/example/Code/xinutec-infra/picade_fleet/install"),
        [(
            "/home/example/Code/xinutec-infra/picade_fleet/install".to_string(),
            false
        )]
    );
}
