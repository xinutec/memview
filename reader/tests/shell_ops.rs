//! What a command *does*, as a type — the layer the file projection reads.
//!
//! These are the facts the older path-and-direction table threw away: what was
//! searched for, what a file used to be called, which script ran. A test here
//! is a claim about meaning; `tests/shell_files.rs` still tests the projection.

use reader::project::read as parse;
use reader::shell_ops::{GitOp, Op, classify};

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
    // ⚠ This was `ffmpeg` until 2026-08-22, when ffmpeg was taught — so the
    // example is now one still on the worklist. Whatever stands here is a
    // placeholder for the next thing to teach, and that is the point of the
    // variant: a gap that names itself can be counted and worked down.
    assert_eq!(
        one("verified_cli --session 2026-06-21"),
        Op::Unknown {
            name: "verified_cli".to_string(),
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
