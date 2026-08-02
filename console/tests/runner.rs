//! A session, driven end to end against a stand-in CLI.
//!
//! What is being tested is the machinery around the process — that a session
//! starts where it is allowed to and nowhere else, that a message reaches it and
//! its answer comes back, that a client which connects late still sees what it
//! missed, and that closing stdin ends it. None of that needs the real CLI, and
//! making it need one would mean a test suite that spends money and fails when
//! the network does.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use console::config::Config;
use console::protocol::Event;
use console::roster::Roster;
use console::session::Spawn;

fn stub() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stub-cli")
        .display()
        .to_string()
}

/// A roster that may start sessions in `dir` and runs the stand-in CLI.
fn roster(dir: &std::path::Path) -> Arc<Roster> {
    Arc::new(Roster::new(Config {
        bind: "127.0.0.1:0".to_string(),
        dirs: vec![dir.to_path_buf()],
        spawn: Spawn {
            binary: stub(),
            model: None,
            permission_mode: None,
        },
        static_dir: None,
    }))
}

/// Wait until the transcript satisfies the predicate, or give up.
///
/// The predicate takes the whole transcript rather than one event because the
/// interesting questions are about counts — "the second turn has finished" is
/// not answerable by looking at events one at a time when the first turn's is
/// still there. Polled rather than slept on, so a slow machine makes this slower
/// instead of flaky.
async fn until(
    session: &Arc<console::session::Session>,
    what: impl Fn(&[Event]) -> bool,
) -> Vec<Event> {
    for _ in 0..100 {
        let history = session.history();
        if what(&history) {
            return history;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "waited 5s and it never happened; transcript was {:?}\nstderr: {}",
        session.history(),
        session.trouble()
    );
}

#[tokio::test]
async fn a_session_starts_takes_a_message_and_answers() {
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");

    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;
    session.send("do the thing").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;

    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::Text { text } if text == "ack")),
        "the answer came back: {seen:?}"
    );
    assert!(
        seen.iter().any(|e| matches!(e, Event::Prompt { .. })),
        "the message was acknowledged: {seen:?}"
    );

    // The summary is what a list of sessions is drawn from, so it has to carry
    // the turn's cost rather than only the transcript.
    let summary = session.summary();
    assert!(summary.alive);
    assert_eq!(summary.turns, 1);
    assert!((summary.cost_usd - 0.25).abs() < f64::EPSILON);
    assert_eq!(summary.asked.as_deref(), Some("do the thing"));
}

#[tokio::test]
async fn one_process_serves_several_turns() {
    // The property the whole design rests on: a session is a conversation, not a
    // series of cold starts.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;

    for turn in 1..=3 {
        session
            .send(&format!("message {turn}"))
            .await
            .expect("send");
        // Count the turns rather than look for one: the previous turn's event
        // is still in the transcript, so "a Turn happened" is already true.
        until(&session, |seen| {
            seen.iter()
                .filter(|e| matches!(e, Event::Turn { .. }))
                .count()
                == turn as usize
        })
        .await;
        let turns = session.summary().turns;
        assert_eq!(turns, turn, "turn {turn} was answered by the same process");
    }
    assert!(session.alive(), "still one live process after three turns");
}

#[tokio::test]
async fn a_late_listener_is_told_what_it_missed() {
    // The phone reconnects on a dropped connection, and must not have to guess
    // what happened while it was away.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("early").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;

    let history = session.history();
    assert!(
        history.iter().any(|e| matches!(e, Event::Started { .. }))
            && history.iter().any(|e| matches!(e, Event::Turn { .. })),
        "the whole turn is replayable to a client that arrived after it: {history:?}"
    );
}

#[tokio::test]
async fn closing_stdin_ends_the_session() {
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;

    session.stop().await;
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Exited { .. }))
    })
    .await;

    assert!(
        seen.iter()
            .any(|e| matches!(e, Event::Exited { code: Some(0) })),
        "it ended cleanly rather than being killed: {seen:?}"
    );
    assert!(!session.alive());
    // And it stays in the roster, because a session that just ended is the one
    // most worth looking at.
    assert!(roster.get(&session.id).is_some());
}

#[test]
fn a_directory_outside_the_allow_list_is_refused() {
    let roster = roster(&std::env::temp_dir().join("nothing-here"));
    let refused = roster.start("/etc").expect_err("/etc is not allowed");
    assert!(refused.contains("/etc"), "says what was refused: {refused}");

    let missing = roster
        .start("/definitely/not/a/directory")
        .expect_err("a path that does not exist is not a session");
    assert!(missing.contains("/definitely/not/a/directory"));
}

#[test]
fn a_symlink_out_of_an_allowed_directory_does_not_escape_it() {
    // The obvious way past a prefix check, and the reason `resolve` canonicalises
    // both sides before comparing.
    let root = std::env::temp_dir().join(format!("console-test-{}", std::process::id()));
    std::fs::create_dir_all(root.join("inside")).expect("temp dirs");
    let link = root.join("inside/escape");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink("/etc", &link).expect("symlink");

    let config = Config {
        bind: "127.0.0.1:0".to_string(),
        dirs: vec![root.join("inside")],
        spawn: Spawn {
            binary: stub(),
            model: None,
            permission_mode: None,
        },
        static_dir: None,
    };
    assert!(
        config.resolve(&link.display().to_string()).is_err(),
        "a symlink pointing out of the allowed tree is refused"
    );
    // Explicit: the test has made its point either way, and a temp dir that
    // will not go is not a reason to fail it.
    let _ = std::fs::remove_dir_all(&root);
}
