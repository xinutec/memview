//! Finding conversations that already happened.
//!
//! The transcripts are read rather than their filenames decoded, so these fixtures
//! are shaped like the real thing in the two ways that matter: the working
//! directory arrives a few lines in rather than on the first, and the directory a
//! transcript claims need not match the name of the folder holding it.

use std::path::Path;

use console::past::{conversations, words_of_claude_processes};

/// A wrapper shell whose *path* says claude — the shape that caused the bug.
/// Claude Code sources a snapshot under `~/.claude/` for every command it runs,
/// so every one of those command lines matches the substring "claude".
const WRAPPER: &str = "/bin/zsh -c source /home/example/.claude/shell-snapshots/snap.sh \
                       && eval 'grep -rn utterance .'";
const REAL: &str = "claude --remote-control health --resume health";

/// Write a transcript whose `cwd` line sits `depth` lines down, as the real ones
/// do — Claude Code opens with a mode line and a session id, and the working
/// directory arrives on a later `system` line.
fn transcript(dir: &Path, project: &str, id: &str, cwd: Option<&str>, depth: usize) {
    let folder = dir.join(project);
    std::fs::create_dir_all(&folder).expect("project dir");
    let mut lines: Vec<String> = (0..depth)
        .map(|n| format!(r#"{{"type":"filler","n":{n}}}"#))
        .collect();
    if let Some(cwd) = cwd {
        lines.push(format!(r#"{{"type":"system","cwd":"{cwd}"}}"#));
    }
    lines.push(r#"{"type":"user"}"#.to_string());
    std::fs::write(folder.join(format!("{id}.jsonl")), lines.join("\n")).expect("transcript");
}

/// A transcript that names itself, the way a real one does: on repeated lines
/// near the end, because a session is renamed as its job changes.
fn named(dir: &Path, id: &str, title: Option<&str>, agent: Option<&str>) {
    let folder = dir.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let mut lines = vec![
        r#"{"type":"mode"}"#.to_string(),
        r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string(),
    ];
    // Named early and renamed later: the later name is the one that counts.
    if let Some(agent) = agent {
        lines.push(format!(r#"{{"type":"agent-name","agentName":"{agent}"}}"#));
    }
    if let Some(title) = title {
        lines.push(format!(
            r#"{{"type":"custom-title","customTitle":"{title}"}}"#
        ));
    }
    std::fs::write(folder.join(format!("{id}.jsonl")), lines.join("\n")).expect("transcript");
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-past-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn a_transcript_is_found_by_the_directory_it_records() {
    // The folder name is deliberately not the encoded form of the cwd: the
    // encoding is undocumented, so nothing may depend on reproducing it.
    let root = scratch("records");
    transcript(
        &root,
        "some-encoded-name",
        "abc-123",
        Some("/home/example/Code/utterance"),
        3,
    );

    let found = conversations(&root);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, "abc-123");
    assert_eq!(found[0].dir, "/home/example/Code/utterance");
    assert!(found[0].bytes > 0);
}

#[test]
fn a_transcript_that_never_says_where_it_ran_is_left_out() {
    // Resuming happens in a directory. One that cannot be identified cannot be
    // resumed safely, and offering it would produce a session in the wrong place.
    let root = scratch("nowhere");
    transcript(&root, "project", "no-cwd", None, 4);

    assert!(conversations(&root).is_empty());
}

#[test]
fn the_working_directory_is_not_on_the_first_line() {
    // The trap this module exists to avoid: a reader that gives up after one line
    // finds nothing, always, and the symptom is an empty list rather than an
    // error.
    let root = scratch("depth");
    transcript(&root, "project", "deep", Some("/home/example/Code"), 8);

    assert_eq!(conversations(&root).len(), 1, "found several lines in");
}

#[test]
fn a_transcript_further_down_than_we_look_is_not_guessed_at() {
    let root = scratch("too-deep");
    transcript(&root, "project", "buried", Some("/home/example/Code"), 200);

    assert!(
        conversations(&root).is_empty(),
        "not found is the honest answer; a default directory would resume somewhere wrong"
    );
}

#[test]
fn the_newest_conversation_comes_first() {
    // The list is for choosing what to pick up again, and the answer is almost
    // always the most recent one.
    let root = scratch("order");
    transcript(&root, "project", "older", Some("/home/example/Code"), 2);
    // Two writes cannot be relied on to differ in mtime, so make them.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    transcript(&root, "project", "newer", Some("/home/example/Code"), 2);

    let found = conversations(&root);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].id, "newer");
}

#[test]
fn files_that_are_not_transcripts_are_ignored() {
    let root = scratch("junk");
    transcript(&root, "project", "real", Some("/home/example/Code"), 2);
    std::fs::write(root.join("project").join("notes.md"), "not a transcript").expect("write");
    std::fs::write(root.join("loose.jsonl"), r#"{"cwd":"/home/example/Code"}"#).expect("write");

    let found = conversations(&root);
    assert_eq!(
        found.len(),
        1,
        "only .jsonl inside a project folder: {found:?}"
    );
    assert_eq!(found[0].id, "real");
}

#[test]
fn a_conversation_is_shown_by_the_name_it_gave_itself() {
    // A hex prefix identifies a transcript; the name identifies the work, which
    // is the thing anybody is actually choosing between.
    let root = scratch("named");
    named(&root, "with-a-name", Some("music"), Some("utterance"));

    let found = conversations(&root);
    assert_eq!(found.len(), 1);
    assert_eq!(
        found[0].name.as_deref(),
        Some("music"),
        "custom-title wins: one is a decision, the other a default"
    );
}

#[test]
fn the_agent_name_is_used_when_nothing_was_set_by_hand() {
    let root = scratch("agent-only");
    named(&root, "auto", None, Some("health"));

    assert_eq!(conversations(&root)[0].name.as_deref(), Some("health"));
}

#[test]
fn a_conversation_that_never_took_a_name_has_none() {
    // Rather than inventing one from the id, which would read as a name and be a
    // hex string wearing a hat.
    let root = scratch("anonymous");
    named(&root, "nameless", None, None);

    assert!(conversations(&root)[0].name.is_none());
}

#[test]
fn a_later_name_replaces_an_earlier_one() {
    // Sessions get renamed as their job changes. The current name is the useful
    // one, which is why the tail is read rather than the head.
    let root = scratch("renamed");
    let folder = root.join("project");
    std::fs::create_dir_all(&folder).expect("dir");
    std::fs::write(
        folder.join("renamed.jsonl"),
        [
            r#"{"type":"system","cwd":"/home/example/Code"}"#,
            r#"{"type":"custom-title","customTitle":"first"}"#,
            r#"{"type":"custom-title","customTitle":"second"}"#,
        ]
        .join("\n"),
    )
    .expect("transcript");

    assert_eq!(conversations(&root)[0].name.as_deref(), Some("second"));
}

#[test]
fn a_transcript_written_moments_ago_is_treated_as_in_use() {
    // The signal that catches a session whose command line says nothing useful.
    // It errs toward busy on purpose: a false "busy" costs a wait, a false "free"
    // costs two processes appending to one transcript.
    let root = scratch("busy-fresh");
    named(&root, "fresh", Some("live"), None);

    assert!(
        conversations(&root)[0].busy,
        "just written, so somebody is probably there"
    );
}

#[test]
fn an_old_transcript_nobody_names_is_free() {
    let root = scratch("busy-old");
    named(&root, "cold", Some("dormant"), None);
    // Backdate it well past the freshness floor.
    let path = root.join("project").join("cold.jsonl");
    let long_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    filetime::set_file_mtime(&path, filetime::FileTime::from_system_time(long_ago))
        .expect("backdate");

    assert!(
        !conversations(&root)[0].busy,
        "old and unnamed by any process"
    );
}

#[test]
fn a_shell_that_merely_mentions_claude_is_not_a_session() {
    // The defect: a conversation named `utterance` was held as in use by any
    // command on this machine containing that word — `grep utterance`, `cd
    // utterance` — because every such command line also carries the path to a
    // shell snapshot under `~/.claude/`.
    assert!(words_of_claude_processes(WRAPPER).is_empty());
}

#[test]
fn a_running_session_contributes_its_arguments() {
    let words = words_of_claude_processes(&format!("{WRAPPER}\n{REAL}\n"));
    assert!(words.iter().any(|word| word == "health"), "{words:?}");
    assert!(
        !words.iter().any(|word| word == "utterance"),
        "the wrapper's words must not leak in: {words:?}"
    );
}

#[test]
fn claude_reached_by_a_full_path_still_counts() {
    let words = words_of_claude_processes("/nix/store/abc/bin/claude --resume music\n");
    assert!(words.iter().any(|word| word == "music"), "{words:?}");
}
