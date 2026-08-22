//! One command, shown as the index reads it.
//!
//! The view's only real failure mode is disagreeing with the miner — looking
//! plausible while the artefact was built from something else. So these are
//! written as claims about *agreement*: the same command, the same directory,
//! the same answer, and the parts of that answer the totals cannot express.

use console::parse::{Asked, parsed};

const HOME: &str = "/home/example";
const CWD: &str = "/home/example/Code/health";

fn asked(command: &str, ok: Option<bool>) -> console::parse::Parsed {
    parsed(
        &Asked {
            command: command.to_string(),
            ok,
        },
        Some(CWD),
        HOME,
    )
}

/// Every path a parse named, with the direction and whether it is certain.
fn uses(command: &str, ok: Option<bool>) -> Vec<(String, bool, bool)> {
    asked(command, ok)
        .steps
        .into_iter()
        .flat_map(|step| step.uses)
        .map(|used| (used.path, used.write, used.certain))
        .collect()
}

#[test]
fn the_view_names_what_the_miner_names() {
    // The agreement that everything else rests on: this is the same walk, so a
    // path that reaches the index reaches the view, resolved the same way.
    let command = "cp src/geo/velocity.ts /tmp/keep.ts";
    let commands = reader::project::read(command).unwrap();
    let mined = reader::shell_files::extract(&commands, Some(CWD), HOME);
    let shown: Vec<(String, bool)> = uses(command, Some(true))
        .into_iter()
        .map(|(path, write, _)| (path, write))
        .collect();
    let expected: Vec<(String, bool)> = mined
        .files
        .into_iter()
        .map(|used| (used.path, used.write))
        .collect();
    assert_eq!(shown, expected);
}

#[test]
fn a_failed_call_leaves_its_conditional_half_unproven() {
    // ⚠ **The reason this view exists.** Both halves parse, both classify, both
    // name the right file — and only one of them certainly happened. Nothing in
    // the command text says which; nothing in the exit status says which; it
    // takes both.
    assert_eq!(
        uses("cat a.ts && cp a.ts b.ts", Some(false)),
        [
            // Before any `&&`, so no exit status can take it away.
            ("/home/example/Code/health/a.ts".to_string(), false, true),
            // After it, and the call failed: this may never have run.
            ("/home/example/Code/health/a.ts".to_string(), false, false),
            ("/home/example/Code/health/b.ts".to_string(), true, false),
        ]
    );
}

#[test]
fn the_same_command_that_succeeded_proves_both() {
    // The other side of the pair — same text, different outcome, and the second
    // half becomes a fact. Asserted separately because a view that ignored the
    // verdict entirely would pass either one alone.
    let certain: Vec<bool> = uses("cat a.ts && cp a.ts b.ts", Some(true))
        .into_iter()
        .map(|(_, _, certain)| certain)
        .collect();
    assert_eq!(certain, [true, true, true]);
}

#[test]
fn a_call_still_running_is_not_a_call_that_failed() {
    // `Unknown` reads as "started, outcome unrecorded". Treating it as failure
    // would empty the view of every session the phone is watching live, which is
    // most of the ones anybody opens this on.
    let certain: Vec<bool> = uses("cat a.ts && cp a.ts b.ts", None)
        .into_iter()
        .map(|(_, _, certain)| certain)
        .collect();
    assert_eq!(certain, [true, false, false]);
}

#[test]
fn a_search_says_what_it_was_looking_for() {
    // The typed operation earning its keep: `grep hsmmDecode src/x.ts` and
    // `cat src/x.ts` project to the same read, and the view must not show them
    // as the same thing.
    let steps = asked("grep -rn hsmmDecode src/geo", Some(true)).steps;
    assert_eq!(steps[0].kind, "search");
    assert_eq!(steps[0].says, "hsmmDecode");
}

#[test]
fn staging_is_shown_as_the_no_op_it_is() {
    // `git add` was 37% of all shell writes before it was dropped. A view that
    // showed it as nothing at all would invite somebody to add it back; showing
    // it as a git step that names no file says why it is right.
    let steps = asked("git add src/geo/velocity.ts", Some(true)).steps;
    assert_eq!(steps[0].kind, "git");
    assert!(steps[0].says.contains("changes no file"));
    assert!(steps[0].uses.is_empty());
}

#[test]
fn a_nested_shell_opens_into_its_own_commands() {
    // A third of the corpus runs through a devshell wrapper. The view's job here
    // is to show that the wrapper is not the thing that touched the file.
    let steps = asked("nix develop -c bash -c 'cp a.ts b.ts'", Some(true)).steps;
    let shape: Vec<(usize, &str)> = steps.iter().map(|step| (step.depth, step.kind)).collect();
    assert_eq!(shape, [(0, "shell"), (1, "copy")]);
    assert!(steps[0].uses.is_empty(), "the wrapper touched nothing");
    assert_eq!(steps[1].uses.len(), 2);
}

#[test]
fn another_machines_file_is_marked_and_never_certain() {
    // The local call's exit status says nothing about a command that ran
    // elsewhere, and the path is not a path on this machine. Both facts have to
    // survive to the screen or the view undoes `RemoteUse` by eye.
    let steps = asked(
        "ssh root@isis.xinutec.org 'cat /etc/nixos/x.nix'",
        Some(true),
    )
    .steps;
    let used = &steps.last().unwrap().uses[0];
    assert_eq!(used.host.as_deref(), Some("isis"));
    assert_eq!(used.path, "/etc/nixos/x.nix");
    assert!(!used.certain);
}

#[test]
fn a_command_that_will_not_parse_says_so() {
    // 113 of 127,342 distinct commands fail to parse (0.09%, 2026-08-15).
    // Returning an empty step list for them would show an unreadable command as
    // a command that did nothing.
    //
    // ⚠ **This was `case $x in a) echo hi;; esac` until memview#901 taught the
    // grammar to read one**, and the test then failed against correct code. An
    // unclosed quote is the durable example: it is not a gap waiting to be
    // filled but text that cannot be read by anything, so nothing will ever
    // close it.
    let answer = asked("grep \"unterminated file.txt", Some(true));
    assert!(answer.error.is_some());
    assert!(answer.steps.is_empty());
}

#[test]
fn an_unread_command_is_named_rather_than_left_blank() {
    // On one command, "what is not in the table" is usually the whole answer to
    // "why did this attribute nothing".
    // ⚠ The example moves as the table grows — `dhall-to-json` stood here until
    // 2026-08-22, when it was taught. What is under test is the naming, not the
    // command.
    let answer = asked("verified_cli --session 2026-06-21", Some(true));
    assert_eq!(answer.unread.len(), 1);
    assert_eq!(answer.unread[0].name, "verified_cli");
}

#[test]
fn a_line_that_is_only_a_redirect_is_still_shown() {
    // It writes a file and classifies to nothing. A view that dropped it would
    // hide a write, which is the one thing it must never do.
    let steps = asked("> /tmp/log", Some(true)).steps;
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].kind, "redirect");
    assert_eq!(steps[0].uses[0].path, "/tmp/log");
    assert!(steps[0].uses[0].write);
}

#[test]
fn a_relative_path_says_which_directory_answered_it() {
    // The question somebody opens this to settle. The text says `main.ts`; the
    // index holds an absolute path; the step is where the two are reconciled.
    let steps = asked("cd frontend && cat main.ts", Some(true)).steps;
    let read = steps.last().unwrap();
    assert_eq!(
        read.cwd.as_deref(),
        Some("/home/example/Code/health/frontend")
    );
    assert_eq!(
        read.uses[0].path,
        "/home/example/Code/health/frontend/main.ts"
    );
}

#[test]
fn what_a_script_runs_is_not_said_twice() {
    // `Op::Run` projects to exactly the script it ran, so naming it again as the
    // step's phrase spends two lines of a 412px screen repeating an absolute
    // path the reader is already looking at.
    let steps = asked("./scripts/deploy.sh --dry-run", Some(true)).steps;
    assert_eq!(steps[0].kind, "run");
    assert_eq!(steps[0].says, "");
    assert_eq!(
        steps[0].uses[0].path,
        "/home/example/Code/health/scripts/deploy.sh"
    );
}

#[test]
fn the_shape_the_phone_is_drawn_from() {
    // ⚠ **Pinned because a render fixture is a copy.** `ui-pages.spec.ts` draws
    // the parse sheet from a hand-written answer for this exact command; if the
    // reader's verdict on it ever moves, this fails rather than leaving the
    // phone check quietly rendering something the runner would never send.
    //
    // The command is the transcript fixture's own, and it FAILED — which is why
    // it is worth drawing: everything after the `&&` parses, classifies and
    // names a real file, and none of it is certain.
    let answer = asked(
        "nix develop -c lake build && ./verified_cli match --serve --timeout 30000ms \
         | tee /tmp/lean-gate.log",
        Some(false),
    );
    let shape: Vec<(&str, &str, bool)> = answer
        .steps
        .iter()
        .map(|step| {
            (
                step.kind,
                step.reached,
                step.uses.iter().all(|used| used.certain),
            )
        })
        .collect();
    assert_eq!(
        shape,
        [
            // The devshell wrapper is unwrapped to `lake build`, which touches
            // no file — so there is nothing to be uncertain about.
            ("nothing", "always", true),
            ("run", "on-success", false),
            ("write", "on-success", false),
        ]
    );
}
