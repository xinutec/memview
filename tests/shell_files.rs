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
    // *shell* either would invent commands nobody ran. Python is read as
    // Python, by a reader that knows it; JavaScript is not read at all, and
    // says so by contributing nothing rather than by contributing nonsense.
    assert_eq!(
        uses("python3 -c 'import os; os.remove(\"src/a.py\")'"),
        [("/home/example/Code/health/src/a.py".to_string(), true)]
    );
    assert!(uses("node -e 'require(\"fs\").readFileSync(\"src/a.ts\")'").is_empty());
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
