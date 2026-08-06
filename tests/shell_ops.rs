//! What a command *does*, as a type — the layer the file projection reads.
//!
//! These are the facts the older path-and-direction table threw away: what was
//! searched for, what a file used to be called, which script ran. A test here
//! is a claim about meaning; `tests/shell_files.rs` still tests the projection.

use memview::shell::parse;
use memview::shell_ops::{GitOp, Op, classify};

const HOME: &str = "/home/example";
const CWD: &str = "/home/example/Code/health";

/// The operations of one script, in running order.
fn ops(script: &str) -> Vec<Op> {
    parse(script)
        .unwrap_or_else(|at| panic!("failed to parse, stopped at {at:?}"))
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
    assert_eq!(
        one("ffmpeg -i in.mp4 out.mp4"),
        Op::Unknown {
            name: "ffmpeg".to_string(),
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
