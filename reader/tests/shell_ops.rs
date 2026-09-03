//! What a command *does*, as a type — the layer the file projection reads.
//!
//! These are the facts the older path-and-direction table threw away: what was
//! searched for, what a file used to be called, which script ran. A test here
//! is a claim about meaning; `tests/shell_files.rs` still tests the projection.

use reader::project::read as parse;
use reader::shell_ops::{GitOp, Op, classify, verb_kind};

const HOME: &str = "/home/example";
const CWD: &str = "/home/example/Code/health";

/// The operations of one script, in running order.
fn ops(script: &str) -> Vec<Op> {
    parse(script)
        .unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"))
        .commands
        .iter()
        .map(|cmd| classify(&cmd.argv, &cmd.heredocs, Some(CWD), HOME))
        .collect()
}

fn one(script: &str) -> Op {
    let mut ops = ops(script);
    assert_eq!(ops.len(), 1, "{script:?} is one command");
    ops.remove(0)
}

#[test]
fn a_search_keeps_what_was_being_looked_for() {
    // The reason for the type. As a path and a direction this is one read,
    // indistinguishable from `cat src/geo/velocity.ts` — and the pattern is the
    // part that says what the agent was actually doing.
    assert_eq!(
        one("grep -rn 'hsmmDecode' src/geo/velocity.ts"),
        Op::Search {
            pattern: "hsmmDecode".to_string(),
            paths: vec!["/home/example/Code/health/src/geo/velocity.ts".to_string()],
        }
    );
}

#[test]
fn a_rename_knows_the_name_a_file_used_to_have() {
    // No count of reads and writes can express this: two paths, one file.
    assert_eq!(
        one("mv src/geo/osm.ts src/geo/overpass.ts"),
        Op::Move {
            from: vec!["/home/example/Code/health/src/geo/osm.ts".to_string()],
            to: "/home/example/Code/health/src/geo/overpass.ts".to_string(),
        }
    );
}

#[test]
fn a_transform_says_whether_it_rewrote_or_only_read() {
    // `-i` is the whole difference, and it is now a field rather than a fork
    // between two table entries.
    assert_eq!(
        one("sed -i '' 's/a/b/' src/geo/osm.ts"),
        Op::Transform {
            program: "s/a/b/".to_string(),
            program_file: None,
            paths: vec!["/home/example/Code/health/src/geo/osm.ts".to_string()],
            in_place: true,
        }
    );
    let Op::Transform { in_place, .. } = one("sed -n '1,40p' src/geo/osm.ts") else {
        panic!("a sed is a transform");
    };
    assert!(!in_place);
}

#[test]
fn a_program_given_as_a_file_is_read_even_when_the_operands_are_written() {
    // It cannot ride along in `paths`, whose direction `in_place` decides —
    // which is why it has a field of its own.
    assert_eq!(
        one("sed -i '' -f scripts/fix.sed src/geo/osm.ts"),
        Op::Transform {
            program: String::new(),
            program_file: Some("/home/example/Code/health/scripts/fix.sed".to_string()),
            paths: vec!["/home/example/Code/health/src/geo/osm.ts".to_string()],
            in_place: true,
        }
    );
}

#[test]
fn a_removal_knows_whether_it_was_recursive() {
    let Op::Remove { recursive, .. } = one("rm -rf build/out") else {
        panic!("an rm is a removal");
    };
    assert!(recursive);
    let Op::Remove { recursive, .. } = one("rm notes/old.md") else {
        panic!("an rm is a removal");
    };
    assert!(!recursive);
}

#[test]
fn running_a_script_names_the_script_and_not_the_interpreter() {
    // The interpreter is not a file anyone works on. Counting the binary put
    // `.venv/bin/python` among the busiest paths in the corpus at 335 reads,
    // which says nothing about anybody's work.
    assert_eq!(
        one("/home/example/Code/recall/.venv/bin/python scripts/report.py --days 7"),
        Op::Run {
            script: "/home/example/Code/health/scripts/report.py".to_string(),
        }
    );
    // A program invoked by path with nothing else to say is still a run of it.
    assert_eq!(
        one("./scripts/verify.sh"),
        Op::Run {
            script: "/home/example/Code/health/scripts/verify.sh".to_string(),
        }
    );
}

#[test]
fn staging_is_named_rather_than_dropped() {
    // `git add` has a variant that exists and projects to no file. Keeping it
    // visible is what stops the decision being silently re-made: counted as a
    // write it was 37% of every shell-derived write in the corpus.
    assert_eq!(
        one("git add src/geo/osm.ts"),
        Op::Git(GitOp::Stage {
            paths: vec!["/home/example/Code/health/src/geo/osm.ts".to_string()],
        })
    );
    assert_eq!(
        one("git commit -m 'x'"),
        Op::Git(GitOp::Other {
            subcommand: "commit".to_string(),
        })
    );
}

#[test]
fn an_unknown_command_names_itself_so_the_gap_can_be_counted() {
    // ⚠ This was `ffmpeg` until 2026-08-22 and `verified_cli` until 2026-08-23,
    // each time because the example got taught. Whatever stands here is a
    // placeholder for the next thing to teach, and that is the point of the
    // variant: a gap that names itself can be counted and worked down. This
    // test failing is the worklist shrinking, not a regression.
    assert_eq!(
        one("home-manager switch --flake .#mac"),
        Op::Unknown {
            name: "home-manager".to_string(),
        }
    );
    // ...and a command that is understood to do nothing is not the same thing.
    assert_eq!(one("echo hello"), Op::Nothing);
}

#[test]
fn a_directory_change_is_an_operation_not_a_file() {
    assert_eq!(
        one("cd ~/Code/memview"),
        Op::ChangeDir {
            to: Some("/home/example/Code/memview".to_string()),
        }
    );
    // An unreadable target is still a directory change, and saying so is what
    // keeps the old directory from staying in force — as `Op::Unknown` did,
    // resolving every later relative path where the script no longer was.
    assert_eq!(one("cd \"$WORKDIR\""), Op::ChangeDir { to: None });
}

#[test]
fn python_is_carried_as_a_program_rather_than_run_as_a_shell() {
    // `-c` holds Python, not shell. Read as shell it invents commands nobody
    // ran; refused entirely, 2,931 calls say nothing at all.
    assert_eq!(
        one("python3 -c 'open(\"x.ts\",\"w\").write(1)'"),
        Op::Python {
            source: "open(\"x.ts\",\"w\").write(1)".to_string(),
        }
    );
    // A heredoc feeding stdin is the same thing written the long way, and is
    // the commoner half of the corpus's Python.
    assert_eq!(
        one("python3 - <<'PY'\nprint(1)\nPY"),
        Op::Python {
            source: "print(1)\n".to_string(),
        }
    );
    assert_eq!(
        one("python3 <<'PY'\nprint(1)\nPY"),
        Op::Python {
            source: "print(1)\n".to_string(),
        }
    );
}

#[test]
fn a_python_script_file_keeps_its_heredoc_as_input() {
    // The body here is the program's *data*, not its source. Reading it as
    // source would attribute the data's paths to a program that never named
    // them.
    assert_eq!(
        one("python3 scripts/load.py <<'ROWS'\nsrc/a.ts\nROWS"),
        Op::Run {
            script: "/home/example/Code/health/scripts/load.py".to_string(),
        }
    );
}

#[test]
fn a_versioned_interpreter_is_still_python() {
    // ⚠ The table matched `python` and `python3` as literals, so `python3.12`
    // produced no `Op::Python` at all — absent from every Python report rather
    // than wrong in one, which is the failure a coverage figure cannot show.
    assert_eq!(
        one("python3.12 -c 'open(\"x.ts\",\"w\").write(1)'"),
        Op::Python {
            source: "open(\"x.ts\",\"w\").write(1)".to_string(),
        }
    );
    assert_eq!(
        one("python3.13 - <<'PY'\nprint(1)\nPY"),
        Op::Python {
            source: "print(1)\n".to_string(),
        }
    );
}

#[test]
fn a_nixpkgs_attribute_is_not_an_interpreter() {
    // `python313` has no dot and is a package name — it appears 68 times in the
    // corpus inside `nix-shell -p python313`, where it runs nothing. Reading it
    // as a call would invent an invocation out of a dependency.
    assert!(!reader::shell_ops::is_python("python313"));
    assert!(!reader::shell_ops::is_python("python-dotenv"));
    assert!(reader::shell_ops::is_python("python"));
    assert!(reader::shell_ops::is_python("python3"));
    assert!(reader::shell_ops::is_python("python3.12"));
}

#[test]
fn a_flag_cluster_still_names_the_script() {
    // ⚠ `-lc` is `-l` and `-c`, not a flag called `lc`. Testing the token for
    // equality with "-c" missed 120 of the corpus's 10,053 shell `-c`
    // invocations, and missed them silently — see `shell_c_value`.
    assert_eq!(
        one("bash -lc 'echo hi'"),
        Op::Nested {
            script: "echo hi".to_string(),
        }
    );
    assert_eq!(
        one("sh -lic 'echo hi'"),
        Op::Nested {
            script: "echo hi".to_string(),
        }
    );
}

#[test]
fn the_commands_that_name_no_file_say_so_rather_than_going_unread() {
    // ⚠ **`Op::Nothing` and `Op::Unknown` are different claims**, and the whole
    // worklist depends on the difference: one says "understood, touches no
    // file", the other says "not read yet". `task` was 12,761 calls at the top
    // of the unread list, three times the next entry, and it is a work queue
    // behind a server with no flag that names a file.
    assert_eq!(one("task edit 1028 --append hello"), Op::Nothing);
    assert_eq!(one("ping -c 2 -W 2 amun.vpn"), Op::Nothing);
    assert_eq!(
        one("journalctl -u restic-backups-cluster.service --no-pager"),
        Op::Nothing
    );
    // Reads every operand, like `cat` — and the corpus's `paste - -` names no
    // path, because a bare dash is not one.
    assert_eq!(one("paste - -"), Op::Read { paths: Vec::new() });
    assert_eq!(
        one("paste a.txt b.txt"),
        Op::Read {
            paths: vec![
                "/home/example/Code/health/a.txt".to_string(),
                "/home/example/Code/health/b.txt".to_string(),
            ],
        }
    );
}

#[test]
fn ffmpeg_reads_its_inputs_and_writes_its_last_operand() {
    assert_eq!(
        one("ffmpeg -hide_banner -i noisy.wav -af afftdn enhanced.wav"),
        Op::Copy {
            from: vec!["/home/example/Code/health/noisy.wav".to_string()],
            to: "/home/example/Code/health/enhanced.wav".to_string(),
        }
    );
    // ⚠ **`-f null -` writes no file, and the guard is what says so** — a
    // reading that took "the last operand" literally would record a write to a
    // file called `-`.
    assert_eq!(
        one("ffmpeg -hide_banner -i in.wav -af volumedetect -f null -"),
        Op::Read {
            paths: vec!["/home/example/Code/health/in.wav".to_string()],
        }
    );
    // A synthetic input is not a file either.
    assert_eq!(
        one("ffmpeg -f lavfi -i anoisesrc=duration=2:amplitude=0.05 sil.wav"),
        Op::Copy {
            from: Vec::new(),
            to: "/home/example/Code/health/sil.wav".to_string(),
        }
    );
}

#[test]
fn an_archives_members_are_not_files_on_this_machine() {
    // ⚠ `FS/data/misc/bluetooth/logs/*` has slashes, so every path test passes
    // it — and it names something INSIDE the zip. Only the archive is a file
    // here, and `-d extracted` is a directory, which this table does not
    // attribute.
    assert_eq!(
        one("unzip -o -q bugreport.zip 'FS/data/misc/bluetooth/logs/*' -d extracted"),
        Op::Read {
            paths: vec!["/home/example/Code/health/bugreport.zip".to_string()],
        }
    );
}

#[test]
fn a_digest_reads_what_it_is_given_and_a_subcommand_is_not_a_path() {
    assert_eq!(
        one("md5 -q dist/console-build/browser/index.html"),
        Op::Read {
            paths: vec![
                "/home/example/Code/health/dist/console-build/browser/index.html".to_string(),
            ],
        }
    );
    // ⚠ `openssl x509 -noout -enddate` reads its certificate from a PIPE, and
    // the guard is what keeps `x509` and `-enddate` from becoming filenames.
    assert_eq!(
        one("openssl x509 -noout -enddate"),
        Op::Read { paths: Vec::new() }
    );
    assert_eq!(
        one("openssl x509 -noout -in cert.pem"),
        Op::Read {
            paths: vec!["/home/example/Code/health/cert.pem".to_string()],
        }
    );
}

#[test]
fn screen_writes_when_it_is_asked_to_and_not_otherwise() {
    // ⚠ The reason that list was checked command by command rather than swept:
    // 222 of `screen`'s calls are `-X hardcopy /tmp/…`, which writes a real
    // file. Filing it under "touches no file" would have deleted those, and
    // nothing downstream could have noticed.
    assert_eq!(
        one("screen -S claude -X hardcopy /tmp/screen.txt"),
        Op::Write {
            paths: vec!["/tmp/screen.txt".to_string()],
        }
    );
    // Everything else it is asked to do touches nothing.
    assert_eq!(
        one("screen -S claude -p 0 -X stuff hello"),
        Op::Write { paths: Vec::new() }
    );
}

#[test]
fn a_converter_that_names_both_ends_in_flags_has_no_operand() {
    // This repository's own gate: the Dhall table is the source, `gate.json` is
    // generated from it, and neither is an operand.
    assert_eq!(
        one("dhall-to-json --file gate.dhall --output gate.json"),
        Op::Copy {
            from: vec!["/home/example/Code/health/gate.dhall".to_string()],
            to: "/home/example/Code/health/gate.json".to_string(),
        }
    );
}

#[test]
fn perl_with_an_in_place_flag_rewrites_its_operands() {
    // ⚠ **3,300 of the corpus's perl calls are this**, and reading `perl` as an
    // interpreter recorded the file it rewrites as a script it RAN — a read,
    // against a file that was being written. The flag is spelled `-0pi` 2,114
    // times and `-pi` 1,028, so a `starts_with("-i")` test sees none of them.
    assert_eq!(
        one(r#"perl -0pi -e 's/a/b/g' src/x.ts"#),
        Op::Transform {
            program: "s/a/b/g".to_string(),
            program_file: None,
            paths: vec!["/home/example/Code/health/src/x.ts".to_string()],
            in_place: true,
        }
    );
    let Op::Transform {
        paths, in_place, ..
    } = one(r#"perl -i -pe 's/a/b/' a.rs b.rs"#)
    else {
        panic!("not a transform");
    };
    assert!(in_place);
    assert_eq!(paths.len(), 2);
}

#[test]
fn perl_without_the_flag_only_reads() {
    // `-ne` carries no `i`, so nothing was rewritten — and the operand is still
    // a file this command opened.
    let Op::Transform {
        paths, in_place, ..
    } = one(r#"perl -ne 'print if /x/' log.txt"#)
    else {
        panic!("not a transform");
    };
    assert!(!in_place);
    assert_eq!(paths, ["/home/example/Code/health/log.txt"]);
}

#[test]
fn a_container_payload_that_is_not_a_shell_stays_an_argv() {
    // ⚠ **memview#1028, and it was 700 of the 769 nested refusals.** `kubectl
    // exec` hands its words to `exec()`; no shell re-splits them and no shell
    // removes a quote. Joining them back into one string and parsing THAT as
    // shell put SQL in front of the shell grammar, where `ROW_COUNT()` reads as
    // unmatched grouping and the whole payload was refused.
    assert_eq!(
        one(
            r#"kubectl -n nextcloud exec deploy/db -- mariadb nc -e "DELETE FROM t WHERE id = 1; SELECT ROW_COUNT() AS deleted""#
        ),
        Op::RemoteRun {
            host: "deploy/db".to_string(),
            argv: vec![
                "mariadb".to_string(),
                "nc".to_string(),
                "-e".to_string(),
                "DELETE FROM t WHERE id = 1; SELECT ROW_COUNT() AS deleted".to_string(),
            ],
        }
    );
}

#[test]
fn a_container_payload_keeps_the_word_boundaries_a_shell_would_have_lost() {
    // The property the argv has and the joined string does not: an argument
    // with spaces in it is ONE argument, still, on the far side.
    let Op::RemoteRun { argv, .. } = one("docker exec c grep -n 'a b c' /etc/hosts") else {
        panic!("not a remote run");
    };
    assert_eq!(argv[2], "a b c");
}

#[test]
fn a_container_payload_in_another_language_reaches_that_language_s_reader() {
    // The point of keeping it an argv: the payload is classified, so a Python
    // one arrives at the Python reader instead of at the shell grammar.
    let Op::RemoteRun { argv, .. } =
        one(r#"kubectl exec pod -- python3 -c 'open("/data/x").read()'"#)
    else {
        panic!("not a remote run");
    };
    assert_eq!(
        reader::shell_ops::classify(&argv, &[], None, "/home/example"),
        Op::Python {
            source: r#"open("/data/x").read()"#.to_string(),
        }
    );
}

#[test]
fn a_clustered_flag_is_not_glued_onto_a_remote_payload() {
    // The shape that exposed it: `kubectl exec … -- sh -lc '…'` handed on
    // `-lc NUM=…` AS the script, so the refusal that followed named the
    // script's own text and never the flag that was really wrong.
    assert_eq!(
        one("kubectl -n signal exec deploy/api -- sh -lc 'curl -s localhost:8080'"),
        Op::Remote {
            host: "deploy/api".to_string(),
            script: "curl -s localhost:8080".to_string(),
        }
    );
}

/// ⚠ **`jq --arg NAME VALUE` consumes TWO words, and skipping one shifted every
/// operand after it.** The value became the first operand and was recorded as
/// the jq program — `2026-01-01` and `5` appear in the corpus census that way —
/// while the real filter fell through to the path list, where its `$name` made
/// it an unnamed file subject. 146 phantom subjects across the corpus, and a
/// program census naming a date.
#[test]
fn a_flag_taking_a_name_and_a_value_eats_both() {
    let argv: Vec<String> = [
        "jq",
        "--arg",
        "dt",
        "2026-01-01",
        ".[] | select(.date==$dt)",
        "d.json",
    ]
    .iter()
    .map(|w| w.to_string())
    .collect();
    let mut unnamed = Vec::new();
    let op = reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    match op {
        reader::shell_ops::Op::Transform { program, paths, .. } => {
            assert_eq!(
                program, ".[] | select(.date==$dt)",
                "the VALUE became the program"
            );
            assert_eq!(paths, ["/home/example/d.json"]);
        }
        other => panic!("not a transform: {other:?}"),
    }
    assert!(
        unnamed.is_empty(),
        "a jq filter was offered as a file subject: {unnamed:?}"
    );
}

/// The plain form must keep working — the fix must not eat a word that is not
/// there.
#[test]
fn a_flag_taking_one_value_still_eats_one() {
    let argv: Vec<String> = ["jq", "-r", ".name", "d.json"]
        .iter()
        .map(|w| w.to_string())
        .collect();
    let mut unnamed = Vec::new();
    let op = reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    match op {
        reader::shell_ops::Op::Transform { program, paths, .. } => {
            assert_eq!(program, ".name");
            assert_eq!(paths, ["/home/example/d.json"]);
        }
        other => panic!("not a transform: {other:?}"),
    }
    assert!(unnamed.is_empty());
}

/// ⚠ **`--slurpfile NAME FILE` loads that file, and the fix must not lose it.**
/// Before `Flags::pair_file`, the flag was unmodelled: its two words fell
/// through as operands, which put the file in the read set BY ACCIDENT and
/// recorded the NAME as jq's program. Skipping both words would have tidied the
/// operands and deleted 32 real reads with them.
#[test]
fn a_pair_flag_whose_second_word_is_a_file_keeps_the_read() {
    let argv: Vec<String> = ["jq", "--slurpfile", "a", "data.json", ".x", "in.json"]
        .iter()
        .map(|w| w.to_string())
        .collect();
    let mut unnamed = Vec::new();
    let op = reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    match op {
        reader::shell_ops::Op::Transform { program, paths, .. } => {
            assert_eq!(program, ".x", "the pair's NAME became the program");
            assert!(
                paths.contains(&"/home/example/data.json".to_string()),
                "the loaded file was dropped: {paths:?}"
            );
            assert!(paths.contains(&"/home/example/in.json".to_string()));
        }
        other => panic!("not a transform: {other:?}"),
    }
}

/// Arithmetic evaluates to a number, so it was never a file subject.
#[test]
fn arithmetic_is_not_a_subject_the_reader_could_not_name() {
    let argv: Vec<String> = ["sed", "-n", "+$((BASE + 1))p", "notes.txt"]
        .iter()
        .map(|w| w.to_string())
        .collect();
    let mut unnamed = Vec::new();
    reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    assert!(
        unnamed.is_empty(),
        "arithmetic counted as a file nobody could name: {unnamed:?}"
    );
}

/// ⚠ **The boundary, and the reason the test is narrow.** A path CONTAINING
/// arithmetic is still a path, and dropping it would delete a real subject to
/// make the opacity figure look better — the trade this whole change exists to
/// avoid.
#[test]
fn a_path_containing_arithmetic_is_still_a_subject() {
    let argv: Vec<String> = ["cat", "/tmp/part-$((n + 1)).txt"]
        .iter()
        .map(|w| w.to_string())
        .collect();
    let mut unnamed = Vec::new();
    reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    assert_eq!(
        unnamed,
        ["/tmp/part-$((n + 1)).txt"],
        "a path with arithmetic in it stopped being a subject"
    );
}

/// A positional parameter genuinely could be a path — whatever the caller
/// passed. It stays counted; only things that could NEVER be a file leave.
#[test]
fn a_positional_parameter_is_still_a_subject() {
    let argv: Vec<String> = ["cat", "$1"].iter().map(|w| w.to_string()).collect();
    let mut unnamed = Vec::new();
    reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    assert_eq!(unnamed, ["$1"]);
}

/// ⚠ **A nested `$(( … ))` must be skipped WHOLE.** Stopping at the first `)`
/// leaves `* c ))` behind, and a stray paren is not in the set of characters an
/// arithmetic operand may leave — so the word would read as a path again. Found
/// while rewriting the scan for a clippy lint: the lint was about style and the
/// version it replaced had this hole.
#[test]
fn nested_arithmetic_is_skipped_whole() {
    let argv: Vec<String> = ["sed", "-n", "$(( (a + b) * c ))p", "notes.txt"]
        .iter()
        .map(|w| w.to_string())
        .collect();
    let mut unnamed = Vec::new();
    reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    assert!(
        unnamed.is_empty(),
        "nested arithmetic read as a file subject: {unnamed:?}"
    );
}

/// A word that spans lines is a program body, not a file subject.
///
/// ⚠ **The population is real and it is one command shape.** Measured
/// 2026-08-23 by `--example body-subjects` over the union corpus: 58 uses, 27
/// distinct, and 56 of them are `perl /tmp/wire.pl <file> '<TypeScript body>'`
/// — a local script whose second argument is source text. It reaches `unnamed`
/// because it carries a `${…}`, which is how a subject the text does not
/// determine is recognised, and a template literal has one for a reason that
/// has nothing to do with the shell.
#[test]
fn a_program_body_spanning_lines_is_not_a_file_subject() {
    let argv: Vec<String> = [
        "perl",
        "/tmp/wire.pl",
        "transcript.ts",
        "  text: `${event.turns ?? 0} turn(s)`,\n  cost: money(event.cost_usd),",
    ]
    .iter()
    .map(|w| w.to_string())
    .collect();
    let mut unnamed = Vec::new();
    let op = reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    assert!(
        unnamed.is_empty(),
        "a program body counted as a file nobody could name: {unnamed:?}"
    );
    // ⚠ **And the file it edits is still credited.** Dropping the body must not
    // cost the operand beside it, which is the only way this fix could pay for
    // a better number with a real read.
    match op {
        reader::shell_ops::Op::Transform { paths, .. } => assert_eq!(
            paths,
            ["/home/example/transcript.ts"],
            "the edited file was lost with the body"
        ),
        other => panic!("not a transform: {other:?}"),
    }
}

/// The boundary: one line with a `$` in it is exactly what this count is for.
#[test]
fn a_single_line_subject_with_a_variable_stays_counted() {
    let argv: Vec<String> = ["cat", "$f"].iter().map(|w| w.to_string()).collect();
    let mut unnamed = Vec::new();
    reader::shell_ops::classify_naming(
        &mut unnamed,
        &argv,
        &[],
        Some("/home/example"),
        "/home/example",
    );
    assert_eq!(unnamed, ["$f"], "the honest floor stopped being counted");
}

/// Helper for the entries below: what one call does, with nothing refused.
fn op_of(words: &[&str]) -> reader::shell_ops::Op {
    let argv: Vec<String> = words.iter().map(|w| w.to_string()).collect();
    reader::shell_ops::classify(&argv, &[], Some("/home/example"), "/home/example")
}

/// `ss` is an interface like `wg`: 294 calls, 20 distinct spellings, and every
/// one is flags — `-tlnp`, `-lnt`, `-ltn`, and one `-tn state established`.
/// Measured 2026-08-23 by `--example unread-shapes`.
#[test]
fn ss_names_no_file() {
    assert!(matches!(
        op_of(&["ss", "-tlnp"]),
        reader::shell_ops::Op::Nothing
    ));
    assert!(matches!(
        op_of(&["ss", "-tn", "state", "established"]),
        reader::shell_ops::Op::Nothing
    ));
}

/// `mysqladmin` is `mariadb-admin` under its old name — 284 calls, 5 distinct
/// spellings, every one a `ping` with connection flags.
///
/// ⚠ **`--socket=/…/mysqld.sock` IS a path**, and it is a path this reading
/// throws away. It survives only because the flag is glued (`--socket=…`), so no
/// operand is left behind; the day one is written `--socket /path`, this entry
/// is where it goes wrong quietly. Same shape as the `wg setconf` note above.
#[test]
fn mysqladmin_names_no_file() {
    assert!(matches!(
        op_of(&["mysqladmin", "ping", "-h", "localhost", "--silent"]),
        reader::shell_ops::Op::Nothing
    ));
}

/// `verified_cli` takes subcommands and reads STDIN — settled from its source
/// (`health/lean/ServeEntry.lean`, `cliMain`), not from how it is called.
/// Every argument is matched by `args.contains` against a verb name or
/// `--timing`; the data arrives through `IO.getStdin` and leaves through
/// stdout. The `< file` in `verified_cli match < legs.json` is the SHELL's
/// redirection, which this layer already reads separately.
#[test]
fn verified_cli_names_no_file() {
    assert!(matches!(
        op_of(&["verified_cli", "match"]),
        reader::shell_ops::Op::Nothing
    ));
    assert!(matches!(
        op_of(&["verified_cli", "--timing", "day"]),
        reader::shell_ops::Op::Nothing
    ));
}

/// `replay` reads one session directory, named positionally.
///
/// ⚠ **The mode flags take NO value, and guessing from their names got this
/// backwards.** `--words <dir>` reads as though `--words` were valued; the
/// source (`scanner/server/src/bin/replay.rs`) shows `--words`, `--slots`,
/// `--tables`, `--pdf`, `--paper`, `--bands` are bare `flag("--x")` tests and
/// the directory is the only positional. Reading the source did not confirm the
/// call-shape reading — it corrected it.
#[test]
fn replay_reads_the_session_directory() {
    match op_of(&["replay", "--words", "/home/example/scan3d_20260716"]) {
        reader::shell_ops::Op::Read { paths } => assert_eq!(
            paths,
            ["/home/example/scan3d_20260716"],
            "the session directory was not read"
        ),
        other => panic!("not a read: {other:?}"),
    }
}

/// ⚠ **The boundary: `--page N` is the one flag that DOES take a value**, and a
/// bare `2` left as an operand would resolve against the cwd into a file called
/// `2` that nothing ever touched.
#[test]
fn replays_page_number_is_not_a_file() {
    match op_of(&["replay", "--page", "2", "/home/example/scan3d_20260716"]) {
        reader::shell_ops::Op::Read { paths } => assert_eq!(
            paths,
            ["/home/example/scan3d_20260716"],
            "the page number was read as a file"
        ),
        other => panic!("not a read: {other:?}"),
    }
}

/// ⚠ **A kind for a caller that has no business with flag tables**, and the
/// obvious way to give it one is a build failure: [`Verb`] carries [`Flags`], so
/// making it public emits `private_interfaces`, and the gate runs
/// `-D warnings`. `verb_kind` is the payload-free half (memview#1364).
#[test]
fn a_command_name_has_a_kind_without_exposing_its_flag_table() {
    assert_eq!(verb_kind("cat"), Some("read"));
    assert_eq!(verb_kind("sed"), Some("stream"));
    assert_eq!(verb_kind("perl"), Some("stream"));
    assert_eq!(verb_kind("rg"), Some("search"));
    assert_eq!(verb_kind("git"), Some("git"));
}

/// ⚠ **`None` is "not taught yet", the same answer [`verb`] gives** — never a
/// bucket a stranger falls into. A concept layer reading this must be able to
/// tell a command the table knows from one it does not.
#[test]
fn a_name_nobody_taught_it_has_no_kind_rather_than_a_default() {
    assert_eq!(verb_kind("frobnicate"), None);
    assert_eq!(verb_kind(""), None);
}

/// ⚠ **The point of the pair: two spellings of one act share a kind.** This is
/// what makes `Rewrite` reachable across `sed -i` and `perl -pi` — measured
/// identical at `Op::Transform { program, in_place }`, and the kind is the
/// coarse half of the same claim.
#[test]
fn two_spellings_of_one_act_share_a_kind() {
    assert_eq!(verb_kind("sed"), verb_kind("perl"));
    assert_eq!(verb_kind("grep"), verb_kind("rg"));
    // And two different acts do not, or the kind would say nothing.
    assert_ne!(verb_kind("cat"), verb_kind("rm"));
}
