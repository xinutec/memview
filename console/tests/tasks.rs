//! The task list a session keeps, read off disk.
//!
//! Fixtures rather than a live `~/.claude`: the shapes tested here are the ones
//! that broke a naive reader — a hundred-and-something sorting before two, a
//! description that is prose rather than a label, and a directory that is not
//! there at all because the session never made a task.

use std::path::Path;

use console::tasks::{detail, listed};

/// One task file, exactly as Claude Code writes it — camel-cased on the wire,
/// which is not this crate's convention and is why the mapping is tested.
fn task(dir: &Path, id: u32, subject: &str, status: &str, description: &str) {
    let json = serde_json::json!({
        "id": id.to_string(),
        "subject": subject,
        "description": description,
        "activeForm": format!("doing {subject}"),
        "status": status,
        "blocks": [],
        "blockedBy": [],
    });
    std::fs::write(
        dir.join(format!("{id}.json")),
        serde_json::to_string(&json).expect("json"),
    )
    .expect("write");
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-tasks-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

fn corpus(name: &str) -> (std::path::PathBuf, String) {
    let root = scratch(name);
    let session = "6f7c2f11-0000-4000-8000-000000000001".to_string();
    let dir = root.join(&session);
    std::fs::create_dir_all(&dir).expect("mkdir");
    task(&dir, 2, "the second", "completed", "how it went");
    task(&dir, 100, "the hundredth", "in_progress", "");
    task(&dir, 101, "the hundred-and-first", "pending", "what to do");
    // Not a task, and not a parse failure either: the directory carries the
    // CLI's own bookkeeping beside the files.
    std::fs::write(dir.join(".highwatermark"), "101").expect("write");
    std::fs::write(dir.join(".lock"), "").expect("write");
    (root, session)
}

#[test]
fn tasks_are_listed_in_the_order_they_were_made() {
    // ⚠ **Numerically.** These are named for their ids, so the directory hands
    // them over with `100` before `2` — and a list that has run past a hundred
    // reads as shuffled, which is exactly when the list is worth opening.
    let (root, session) = corpus("order");

    let all = listed(&root, &session);

    assert_eq!(
        all.iter().map(|it| it.id.as_str()).collect::<Vec<_>>(),
        ["2", "100", "101"]
    );
}

#[test]
fn the_cli_s_own_status_words_travel_rather_than_a_flag_of_ours() {
    // Three states, not two: a client sorts what is underway above what is
    // merely open, and an `open: bool` would have thrown that away.
    let (root, session) = corpus("statuses");

    let all = listed(&root, &session);

    assert_eq!(
        all.iter().map(|it| it.status.as_str()).collect::<Vec<_>>(),
        ["completed", "in_progress", "pending"]
    );
    assert_eq!(all[0].active_form.as_deref(), Some("doing the second"));
}

#[test]
fn a_task_with_nothing_written_up_does_not_offer_to_open() {
    // The list says whether there is prose behind a row, so the client can
    // decline to make it tappable — a sheet that opens onto nothing is worse
    // than a row that does not open.
    let (root, session) = corpus("detailed");

    let all = listed(&root, &session);

    assert!(all[0].detailed, "a task with a description");
    assert!(!all[1].detailed, "a task written as a one-line reminder");
}

#[test]
fn what_a_task_says_is_fetched_on_its_own() {
    // ⚠ **The reason there are two requests.** Descriptions are written-up
    // results running to kilobytes — one live session's 355 tasks are 1.5 MB of
    // them, which is not a payload for drawing forty subjects on a phone.
    let (root, session) = corpus("detail");

    assert_eq!(detail(&root, &session, "2").as_deref(), Some("how it went"));
    assert_eq!(detail(&root, &session, "404"), None);
}

#[test]
fn a_session_that_never_made_a_task_has_an_empty_list_rather_than_a_failure() {
    // The ordinary case: most conversations never open a task list at all, and
    // several here have only the lockfile. No directory is no tasks.
    let root = scratch("empty");

    assert_eq!(listed(&root, "never-made-one"), Vec::new());
}

#[test]
fn a_file_that_will_not_parse_is_skipped_rather_than_taken_as_a_blank_task() {
    // The CLI owns this format. A shape we do not know is news — logged — and
    // must not become a row with no subject sitting in the middle of the list.
    let (root, session) = corpus("unparsable");
    std::fs::write(root.join(&session).join("7.json"), "{\"id\":").expect("write");

    let all = listed(&root, &session);

    assert_eq!(all.len(), 3, "the three readable ones, and no blank fourth");
}
