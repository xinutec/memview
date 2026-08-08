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
