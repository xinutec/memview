//! What a call's own output says about it — against the wordings the
//! transcripts actually contain.
//!
//! The negative cases carry as much weight as the positive: this reads prose
//! that a shell happens to have printed, and a false positive silently *stops*
//! the parser applying a `cd` that really happened.

use reader::doing::refused_dirs;

#[test]
fn the_shell_names_the_target_it_would_not_enter() {
    assert_eq!(
        refused_dirs("cd: memcheck: No such file or directory"),
        vec!["memcheck".to_string()],
    );
}

#[test]
fn bashs_own_prefix_is_not_part_of_the_target() {
    // The wording these sessions produce, in full — `SHELL` is bash, so this is
    // the form 247 results in this project's transcripts carry.
    let said = "/nix/store/…-bash-interactive-5.3p9/bin/bash: line 1: \
                cd: memcheck: No such file or directory";
    assert_eq!(refused_dirs(said), vec!["memcheck".to_string()]);
}

#[test]
fn a_path_with_its_own_colons_is_read_from_the_last_cd() {
    assert_eq!(
        refused_dirs("bash: line 1: cd: frontend/projects/console-web: No such file or directory"),
        vec!["frontend/projects/console-web".to_string()],
    );
}

#[test]
fn every_refusal_in_one_output_is_read_not_only_the_last() {
    let said = "cd: lean: No such file or directory\n\
                make: nothing to be done\n\
                cd: android: No such file or directory";
    assert_eq!(
        refused_dirs(said),
        vec!["lean".to_string(), "android".to_string()],
    );
}

#[test]
fn a_target_that_is_a_file_rather_than_a_directory_is_still_refused() {
    assert_eq!(
        refused_dirs("cd: Cargo.toml: Not a directory"),
        vec!["Cargo.toml".to_string()],
    );
}

#[test]
fn a_trailing_slash_is_not_part_of_the_name() {
    // `cd frontend/` is echoed back with the slash; the operand in the script is
    // matched against this, and the two must agree.
    assert_eq!(
        refused_dirs("cd: frontend/: No such file or directory"),
        vec!["frontend".to_string()],
    );
}

#[test]
fn prose_that_merely_begins_with_the_prefix_is_not_a_refusal() {
    // ⚠ Straight from the corpus: `git log --oneline` output, where a commit
    // subject begins with `cd: `. Read as a refusal, this would stop a `cd` that
    // succeeded from being applied — the same defect, pointing the other way.
    let said = "8047bc0 cd: TLS handshake + banner (amun.xinutec.org)\n\
                b359839 cd: harden inspircd (runAsNonRoot uid 39 + drop ALL)";
    assert!(refused_dirs(said).is_empty());
}

#[test]
fn a_mention_of_the_ending_without_a_cd_is_not_a_refusal() {
    // Every other command reports the same ending, and none of them moved the
    // directory. `cat` failing says nothing about where the script is.
    assert!(refused_dirs("cat: nope.txt: No such file or directory").is_empty());
}

#[test]
fn silence_is_the_ordinary_case() {
    assert!(refused_dirs("").is_empty());
    assert!(refused_dirs("total 0\ndrwxr-xr-x  2 example  staff  64 Aug  8 17:00 .").is_empty());
}

#[test]
fn zsh_names_the_target_after_the_message_and_is_read_too() {
    // ⚠ **43% of the corpus's refusals were invisible** while only bash's
    // wording was read: measured 2026-08-12, **74 calls say it zsh's way and 99
    // bash's**. Every missed one is a `cd` the parser applied and the shell did
    // not — the exact defect this function exists to prevent.
    assert_eq!(
        refused_dirs("cd:1: no such file or directory: src"),
        vec!["src".to_string()],
    );
}

#[test]
fn a_zsh_refusal_from_inside_eval_is_read_too() {
    // How it usually arrives: `nix develop -c`, `nix-shell --run` and `ssh`
    // one-liners reach the shell through `eval`, which prefixes its own name.
    // This is the commonest form in the corpus.
    assert_eq!(
        refused_dirs("(eval):cd:1: no such file or directory: frontend/src/app/services"),
        vec!["frontend/src/app/services".to_string()],
    );
}

#[test]
fn zsh_says_not_a_directory_the_same_way() {
    assert_eq!(
        refused_dirs("(eval):cd:1: not a directory: Cargo.toml"),
        vec!["Cargo.toml".to_string()],
    );
}

#[test]
fn one_output_may_carry_both_shells() {
    // A script that runs a nested shell reports in whichever one refused, and a
    // call can contain more than one.
    let said = "bash: line 1: cd: memcheck: No such file or directory\n\
                (eval):cd:1: no such file or directory: src";
    assert_eq!(
        refused_dirs(said),
        vec!["memcheck".to_string(), "src".to_string()],
    );
}

#[test]
fn zsh_prose_in_the_same_position_is_still_not_a_refusal() {
    // The message has to sit where zsh puts it. A commit subject that merely
    // contains the words does not.
    let said = "8047bc0 cd: fix no such file or directory: handling in the reader";
    assert!(refused_dirs(said).is_empty());
}

// ── Resuming a fold (#1240) ──────────────────────────────────────────────────
//
// The mine is 347 s over 5.9 GB and only reading less makes it cheaper, which
// means a run has to be able to pick up where the last one stopped. These say
// what that carries exactly — and what it does not.

use reader::doing::{Log, Names, Verdict, Work};

fn work<'a>(call: &'a str, agent: &'a str, kind: &'a str, minute: i64) -> Work<'a> {
    Work {
        call,
        agent,
        project: Some("memview"),
        host: None,
        kind,
        n: 1,
        minute,
    }
}

/// The contract the whole resume rests on: an index written into a carried row
/// still names what it named. A dictionary that re-interned in any other order
/// would leave every row pointing at the wrong agent, silently.
#[test]
fn a_rebuilt_dictionary_gives_back_the_indices_it_was_frozen_with() {
    let mut names = Names::default();
    let first = names.intern("memview");
    let second = names.intern("home");
    names.intern("memview");
    let frozen = names.into_vec();

    let mut back = Names::from_vec(frozen);
    assert_eq!(back.intern("memview"), first);
    assert_eq!(back.intern("home"), second);
    // A name it has not seen continues the numbering rather than colliding.
    assert_eq!(back.intern("tasks"), 2);
}

/// Reading a transcript in two goes must produce what reading it in one go
/// produces. This is the claim; the two tests after it are the exceptions.
#[test]
fn a_fold_split_in_two_says_what_one_pass_says() {
    let whole = {
        let mut log = Log::default();
        log.open_transcript();
        log.begin_episode("memview");
        log.push(work("c1", "memview", "Read", 10));
        log.resolve("c1", Verdict::Ok);
        log.push(work("c2", "memview", "Edit", 11));
        log.resolve("c2", Verdict::Failed);
        log.finish("stamp")
    };

    let split = {
        let mut first = Log::default();
        first.open_transcript();
        first.begin_episode("memview");
        first.push(work("c1", "memview", "Read", 10));
        first.resolve("c1", Verdict::Ok);
        let (episode, prompt) = first.open_episode();
        let frozen = first.finish("stamp");

        let mut second = Log::resume(frozen);
        second.reopen(episode, prompt);
        second.push(work("c2", "memview", "Edit", 11));
        second.resolve("c2", Verdict::Failed);
        second.finish("stamp")
    };

    assert_eq!(
        serde_json::to_string(&whole).expect("whole"),
        serde_json::to_string(&split).expect("split"),
    );
}

/// ⚠ **Without the carried episode the second half is orphaned.** This is the
/// 66-calls-a-run loss `Resume` exists to prevent, written down as a failure so
/// that dropping `reopen` from the miner cannot pass the suite.
#[test]
fn resuming_without_the_open_episode_orphans_the_rows_after_the_cut() {
    let mut first = Log::default();
    first.open_transcript();
    first.begin_episode("memview");
    first.push(work("c1", "memview", "Read", 10));
    let frozen = first.finish("stamp");

    let mut second = Log::resume(frozen);
    // No `reopen`: exactly what a resume that carried only the byte offset does.
    second.push(work("c2", "memview", "Edit", 11));
    let out = second.finish("stamp");

    assert_eq!(out.rows[0].e, Some(0));
    assert_eq!(
        out.rows[1].e, None,
        "the row after the cut lost its episode"
    );
    // And the episode never learns it grew, so a page reporting its size lies.
    assert_eq!(out.episodes[0].n, 1);
    assert_eq!(out.episodes[0].until, 10);
}

/// ⚠ **The one thing a resume drops on purpose.** A call answered after the cut
/// has nothing left to match its result to, so its verdict stays unknown.
/// Measured at 3 calls across the corpus against a 349-row baseline — small
/// enough to accept, not small enough to leave unsaid.
#[test]
fn a_call_answered_after_the_cut_keeps_no_verdict() {
    let mut first = Log::default();
    first.open_transcript();
    first.begin_episode("memview");
    first.push(work("c1", "memview", "Bash", 10));
    let frozen = first.finish("stamp");

    let mut second = Log::resume(frozen);
    second.resolve("c1", Verdict::Ok);
    let out = second.finish("stamp");
    assert_eq!(out.rows[0].v, Verdict::Unknown);
}
