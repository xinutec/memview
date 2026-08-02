//! Which files a command used — against the commands the transcripts contain.
//!
//! Every case here is a shape taken from the corpus, and the negative ones carry
//! as much weight as the positive: this table's only real failure mode is
//! putting a path into an agent's record that the command never named.

use memview::shell::parse;
use memview::shell_files::extract;

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
    assert_eq!(
        uses("for f in a b; do cat src/geo/osm.ts; done"),
        [(
            "/home/example/Code/health/src/geo/osm.ts".to_string(),
            false
        )]
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
    assert!(uses("ffmpeg -i in.mp4 out.mp4").is_empty());
    assert_eq!(unread("ffmpeg -i in.mp4 out.mp4"), ["ffmpeg"]);
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
