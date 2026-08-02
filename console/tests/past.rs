//! Finding conversations that already happened.
//!
//! The transcripts are read rather than their filenames decoded, so these fixtures
//! are shaped like the real thing in the two ways that matter: the working
//! directory arrives a few lines in rather than on the first, and the directory a
//! transcript claims need not match the name of the folder holding it.

use std::path::Path;

use console::past::conversations;

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
