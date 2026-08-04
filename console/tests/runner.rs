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
        desk: "127.0.0.1:0".to_string(),
        tls: None,
        dirs: vec![dir.to_path_buf()],
        spawn: Spawn {
            binary: stub(),
            model: None,
            permission_mode: None,
        },
        static_dir: None,
        // No dashboard in a test: the front page is drawn without usage on it.
        usage_url: None,
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
    }
    assert!(session.alive(), "still one live process after three turns");

    // ⚠ **`total_cost_usd` is already the session total, so it must not be
    // added up.** Summing totals gives a triangular sum — on a live session it
    // reached $59.32 against a true $12.35 — and it looks perfectly plausible
    // while doing it. The stub reports a rising total (0.25, 0.50, 0.75) the way
    // the CLI does, so this can fail.
    let summary = session.summary();
    assert!(
        (summary.cost_usd - 0.75).abs() < 1e-9,
        "the cost is the latest total, not the sum of the totals: {}",
        summary.cost_usd
    );
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
async fn a_reconnecting_listener_is_sent_only_what_it_missed() {
    // The reason this exists: the page had no way to say where it had got to, so
    // every reconnect meant replaying the whole transcript and the client
    // throwing away everything it held — including pages it had scrolled back to
    // load, which are not in this log at all and cannot be recovered.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("early").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;

    // Where a client watching from the start would have got to.
    let caught_up = session.since(None);
    assert!(
        !caught_up.resumed,
        "a client naming nothing is owed everything"
    );
    let held = caught_up.through;
    assert!(held > 0, "the first turn produced events: {caught_up:?}");

    // Nothing happened while it was away.
    let quiet = session.since(Some(held));
    assert!(quiet.resumed);
    assert!(
        quiet.events.is_empty(),
        "nothing to catch up on: {:?}",
        quiet.events
    );
    assert_eq!(quiet.through, held, "and it is still where it was");

    // A second turn happens, and only that turn is owed.
    session.send("later").await.expect("send");
    until(&session, |seen| {
        seen.iter()
            .filter(|e| matches!(e, Event::Turn { .. }))
            .count()
            == 2
    })
    .await;

    let missed = session.since(Some(held));
    assert!(missed.resumed);
    assert!(
        missed.events.iter().all(|stamped| stamped.seq > held),
        "nothing it already had: {:?}",
        missed.events
    );
    assert!(
        missed
            .events
            .iter()
            .any(|stamped| matches!(stamped.event, Event::Turn { .. })),
        "and everything it did not: {:?}",
        missed.events
    );
    assert!(
        missed.events.len() < session.history().len(),
        "a resume is a tail, not the transcript again"
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
        desk: "127.0.0.1:0".to_string(),
        tls: None,
        dirs: vec![root.join("inside")],
        spawn: Spawn {
            binary: stub(),
            model: None,
            permission_mode: None,
        },
        static_dir: None,
        // No dashboard in a test: the front page is drawn without usage on it.
        usage_url: None,
    };
    assert!(
        config.resolve(&link.display().to_string()).is_err(),
        "a symlink pointing out of the allowed tree is refused"
    );
    // Explicit: the test has made its point either way, and a temp dir that
    // will not go is not a reason to fail it.
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn a_question_blocks_the_session_until_it_is_answered() {
    // The property approvals exist for: the session stops, says what it wants to
    // run, and does nothing until a person decides.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;

    session.send("may i run something").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;

    let Some(Event::Ask { id, tool, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. }))
    else {
        panic!("no question in {seen:?}");
    };
    assert_eq!(tool, "Bash");
    // Nothing has happened yet, and the summary says so — this is what a list of
    // sessions shows as "waiting for you".
    assert_eq!(session.summary().waiting, 1);
    assert!(
        !seen.iter().any(|e| matches!(e, Event::Turn { .. })),
        "the turn has not finished while the question stands: {seen:?}"
    );

    session.decide(id, true, "", None).await.expect("approve");
    let after = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    assert!(
        after
            .iter()
            .any(|e| matches!(e, Event::Text { text } if text == "allowed")),
        "the session was told it may: {after:?}"
    );
    assert_eq!(session.summary().waiting, 0, "no longer waiting");
}

#[tokio::test]
async fn a_refusal_carries_a_reason_and_the_session_goes_on() {
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("may i run something").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    session
        .decide(id, false, "not that directory", None)
        .await
        .expect("refuse");
    let after = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    assert!(
        after
            .iter()
            .any(|e| matches!(e, Event::Text { text } if text == "refused")),
        "the refusal reached the session: {after:?}"
    );
}

#[tokio::test]
async fn a_choice_reaches_the_session_as_part_of_what_it_asked() {
    // **The property that makes questions answerable at all.** `AskUserQuestion`
    // reads the answers out of its own arguments, so approving it unchanged says
    // nothing — the console has to hand back an input it has written the choice
    // into. The stub reports what it received, so what is asserted here is what
    // the CLI would see, not what we meant to send.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("ask me which way").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, tool, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. }))
    else {
        panic!("no question in {seen:?}");
    };
    assert_eq!(tool, console::protocol::QUESTION_TOOL);

    let reply = console::protocol::Reply {
        annotations: console::protocol::Annotations::default(),
        answers: console::protocol::Answers::from([(
            "which way".to_string(),
            console::protocol::Answer::One("left".to_string()),
        )]),
        response: None,
    };
    session
        .decide(id, true, "", Some(&reply))
        .await
        .expect("answer");
    let after = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    assert!(
        after
            .iter()
            .any(|e| matches!(e, Event::Text { text } if text == "chose left")),
        "the choice arrived inside the tool's own arguments: {after:?}"
    );
}

#[tokio::test]
async fn what_was_chosen_is_told_to_everybody_watching() {
    // The client that tapped already knows. A second screen on the same session,
    // and the same screen after a reload, do not — and an `ask` is a control
    // request rather than a transcript line, so nothing can hand it back later.
    // The verdict event is the only place that knows it for all of them.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("ask me which way").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    let reply = console::protocol::Reply {
        annotations: console::protocol::Annotations::default(),
        answers: console::protocol::Answers::from([(
            "which way".to_string(),
            console::protocol::Answer::One("left".to_string()),
        )]),
        response: None,
    };
    session
        .decide(id, true, "", Some(&reply))
        .await
        .expect("answer");

    // Read from the history, which is what a client arriving late is sent.
    let history = session.history();
    let Some(Event::Answered {
        reply: Some(told), ..
    }) = history.iter().find(|e| matches!(e, Event::Answered { .. }))
    else {
        panic!("the verdict said nothing about what was chosen: {history:?}");
    };
    assert_eq!(told, &reply);
}

#[tokio::test]
async fn words_instead_of_a_choice_travel_on_their_own() {
    // ⚠ **`response` overrides `answers` in the CLI, so the two are never both
    // sent.** Its result builder tests `response` first and reports only that —
    // prose alongside a set of choices would throw the choices away silently.
    // The stub reports whichever it received, so this fails if both go.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("ask me which way").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    let reply = console::protocol::Reply {
        annotations: console::protocol::Annotations::default(),
        answers: console::protocol::Answers::default(),
        response: Some("neither, go back".to_string()),
    };
    session
        .decide(id, true, "", Some(&reply))
        .await
        .expect("reply");
    let after = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    assert!(
        after
            .iter()
            .any(|e| matches!(e, Event::Text { text } if text == "said neither, go back")),
        "the words arrived, and no answers with them: {after:?}"
    );
}

#[tokio::test]
async fn a_note_rides_with_the_choice_rather_than_replacing_it() {
    // ⚠ **The difference from `response`.** Words in the reply field override
    // the choices; a note qualifies one. The CLI reports
    // `"<question>"="<label>" notes: <notes>`, so both have to arrive.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("ask me which way").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    let reply = console::protocol::Reply {
        answers: console::protocol::Answers::from([(
            "which way".to_string(),
            console::protocol::Answer::One("left".to_string()),
        )]),
        response: None,
        annotations: console::protocol::Annotations::from([(
            "which way".to_string(),
            console::protocol::Annotation {
                notes: Some("but slowly".to_string()),
            },
        )]),
    };
    session
        .decide(id, true, "", Some(&reply))
        .await
        .expect("answer");
    let after = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    assert!(
        after
            .iter()
            .any(|e| matches!(e, Event::Text { text } if text == "chose left noting but slowly")),
        "the note arrived beside the choice, not instead of it: {after:?}"
    );
}

#[tokio::test]
async fn a_note_with_nothing_blank_in_it_is_all_that_travels() {
    // A field left empty is not a note. The CLI tests `notes` for truthiness and
    // reports `(no option selected)` for any question it finds one against — so
    // a blank one would invent an answer to a question nobody touched.
    let empty = console::protocol::Reply {
        answers: console::protocol::Answers::default(),
        response: None,
        annotations: console::protocol::Annotations::from([(
            "which way".to_string(),
            console::protocol::Annotation {
                notes: Some("   ".to_string()),
            },
        )]),
    };
    assert!(empty.is_empty(), "whitespace is not something said");

    let real = console::protocol::Reply {
        annotations: console::protocol::Annotations::from([(
            "which way".to_string(),
            console::protocol::Annotation {
                notes: Some("neither, really".to_string()),
            },
        )]),
        ..Default::default()
    };
    assert!(!real.is_empty(), "a note on its own is an answer");
}

#[tokio::test]
async fn an_empty_reply_is_an_ordinary_approval() {
    // The route hands every allow through the same field, so "nothing was said"
    // has to stay distinguishable from "an answer was given" — otherwise every
    // approved Bash call would look like an edited one and be refused.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("may i run something").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    let nothing = console::protocol::Reply::default();
    assert!(nothing.is_empty());
    session
        .decide(id, true, "", None)
        .await
        .expect("an ordinary approval still works");
}

#[tokio::test]
async fn only_a_question_may_have_its_arguments_edited() {
    // `updatedInput` would let a client approve a *different* command from the
    // one it was shown. The console answers questions with it and nothing else,
    // so a console that is compromised still cannot rewrite what it approves.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("may i run something").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    let reply = console::protocol::Reply {
        annotations: console::protocol::Annotations::default(),
        answers: console::protocol::Answers::from([(
            "command".to_string(),
            console::protocol::Answer::One("rm -rf /".to_string()),
        )]),
        response: None,
    };
    let refused = session.decide(id, true, "", Some(&reply)).await;
    assert!(refused.is_err(), "answers are refused for a Bash call");
    assert!(
        format!("{:#}", refused.unwrap_err()).contains("Bash"),
        "and names the tool that does not ask questions"
    );
    // Still open: a rejected edit is not an answer, and the session is waiting.
    assert_eq!(session.summary().waiting, 1);
}

#[tokio::test]
async fn a_question_can_only_be_answered_once() {
    // Two people can have the same session open. The second answer must be
    // refused rather than sent, or the CLI is told twice about one question.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("may i run something").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    session
        .decide(id, true, "", None)
        .await
        .expect("first answer");
    let again = session.decide(id, false, "changed my mind", None).await;
    assert!(again.is_err(), "the second answer is refused");
    assert!(
        format!("{:#}", again.unwrap_err()).contains("already"),
        "and says why"
    );
}

#[tokio::test]
async fn a_session_keeps_the_pipes_an_upgrade_would_carry() {
    // The upgrade hands three descriptor numbers and a pid to the next image.
    // If any is missing there is nothing to carry, and the session would be
    // silently unreachable on the other side rather than visibly dropped.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");

    let fds = session.fds();
    assert!(
        fds.stdin >= 0 && fds.stdout >= 0 && fds.stderr >= 0,
        "{fds:?}"
    );
    assert_ne!(
        session.pid(),
        0,
        "a pid is the only handle an adopted session has"
    );
}

#[tokio::test]
async fn a_live_pipe_can_be_taken_out_of_close_on_exec() {
    // ⚠ Rust marks every pipe it creates close-on-exec, so without this the
    // upgraded image inherits nothing at all — the failure would be total and
    // silent, which is why it is asserted rather than assumed.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    let fds = session.fds();

    for fd in [fds.stdin, fds.stdout, fds.stderr] {
        assert!(
            console::session::keepable(fd),
            "fd {fd} could not be carried"
        );
        // SAFETY: reading a flag on a descriptor this process owns.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
        assert_eq!(flags & libc::FD_CLOEXEC, 0, "fd {fd} would close on exec");
    }
}

/// A pipe whose ends are both fit to be carried across an upgrade.
///
/// Returns `(read, write)`. Both ends are kept open by the caller: closing the
/// far end would make the adopted session read end of file at once and call
/// itself over.
fn carried_pipe() -> (std::os::fd::RawFd, std::os::fd::RawFd) {
    let mut ends = [0 as libc::c_int; 2];
    // SAFETY: pipe(2) writes two descriptors into an array this test owns.
    assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0, "pipe");
    for end in ends {
        assert!(console::session::keepable(end), "fd {end}");
    }
    (ends[0], ends[1])
}

#[tokio::test]
async fn an_adopted_session_carries_the_numbers_no_transcript_holds() {
    // ⚠ **The result line is never written to disk.** It is what carries the
    // cost, the rate-limit status and the window — and grepping
    // the whole corpus for `"type":"result"` finds none, so [`seed`] cannot get
    // these back the way it gets the conversation back. Either the previous
    // image hands them over or an upgraded session reads as a fresh one that has
    // done nothing: no model, and a context with no window to be a
    // fraction of. That is exactly what the phone showed after the first
    // upgrade.
    let tally = console::session::Tally {
        started: 1_754_000_000,
        model: Some("claude-opus-5".into()),
        mode: Some("auto".into()),
        cost_usd: 1.25,
        window: Some(1_000_000),
        limit: Some("allowed_warning".into()),
        pending: Default::default(),
        spent: Default::default(),
        picked: None,
    };
    let (_stdin_read, stdin) = carried_pipe();
    let (stdout, _stdout_write) = carried_pipe();
    let (stderr, _stderr_write) = carried_pipe();

    let session = console::session::Session::adopt(
        // No transcript by this name, so nothing seeded can account for the
        // numbers below — they can only have come across the handover.
        "not-a-session-on-disk".into(),
        std::env::temp_dir(),
        std::process::id(),
        console::session::Fds {
            stdin,
            stdout,
            stderr,
        },
        tally.clone(),
    )
    .expect("adopt");

    let summary = session.summary();
    assert_eq!(summary.started, 1_754_000_000, "the session looks newborn");
    assert_eq!(summary.mode.as_deref(), Some("auto"), "the mode restarted");
    assert_eq!(summary.model.as_deref(), Some("claude-opus-5"));
    assert_eq!(summary.window, Some(1_000_000), "no window to be full of");
    assert_eq!(summary.limit.as_deref(), Some("allowed_warning"));
    assert!((summary.cost_usd - 1.25).abs() < f64::EPSILON);
}

#[tokio::test]
async fn an_upgrade_keeps_the_question_a_session_is_blocked_on() {
    // ⚠ **The defect this exists for, measured on a live session.** `execve`
    // does not touch the child, so a session blocked on `can_use_tool` is STILL
    // blocked after an upgrade — but the pending request lived only in the image
    // that was replaced. It was dropped, and a control request is not a
    // transcript line, so the re-seed could not put it back either: the card
    // vanished off every screen, the request id ceased to exist anywhere, and
    // the session waited for an answer that had become unsendable. One
    // conversation sat on "running" for an hour that way.
    let asked = console::session::Pending {
        tool: "AskUserQuestion".into(),
        input: serde_json::json!({"questions": [{"question": "which way"}]}),
        title: None,
        detail: Some("a question standing when the console was replaced".into()),
    };
    let tally = console::session::Tally {
        started: 1_754_000_000,
        model: Some("claude-opus-5".into()),
        mode: Some("auto".into()),
        cost_usd: 0.0,
        window: None,
        limit: None,
        pending: std::collections::BTreeMap::from([("ask-1".to_string(), asked)]),
        spent: Default::default(),
        picked: None,
    };
    let (_stdin_read, stdin) = carried_pipe();
    let (stdout, _stdout_write) = carried_pipe();
    let (stderr, _stderr_write) = carried_pipe();

    let session = console::session::Session::adopt(
        "not-a-session-on-disk".into(),
        std::env::temp_dir(),
        std::process::id(),
        console::session::Fds {
            stdin,
            stdout,
            stderr,
        },
        tally,
    )
    .expect("adopt");

    // The list says it is waiting, which is what makes it findable at all.
    assert_eq!(session.summary().waiting, 1, "the question was dropped");
    // And a client is offered the decision again, rather than being left with a
    // tool row that never finishes.
    let history = session.history();
    let Some(console::protocol::Event::Ask { id, tool, .. }) = history
        .iter()
        .find(|e| matches!(e, console::protocol::Event::Ask { .. }))
    else {
        panic!("nothing on screen to answer: {history:?}");
    };
    assert_eq!(id, "ask-1", "the id is what the answer has to carry back");
    assert_eq!(tool, "AskUserQuestion");

    // Answerable, which is the whole point: the id still means something.
    let reply = console::protocol::Reply {
        answers: console::protocol::Answers::from([(
            "which way".to_string(),
            console::protocol::Answer::One("left".to_string()),
        )]),
        ..Default::default()
    };
    session
        .decide("ask-1", true, "", Some(&reply))
        .await
        .expect("the carried question can still be answered");
    assert_eq!(session.summary().waiting, 0);
}

#[tokio::test]
async fn a_session_hands_on_the_question_it_is_blocked_on() {
    // The other half of the same property, and the half the test above cannot
    // see: it builds a `Tally` by hand, so it would still pass if `tally()`
    // stopped reading `pending` off a live session. Measured by ablation.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("may i run something").await.expect("send");
    let seen = until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    let Some(Event::Ask { id, .. }) = seen.iter().find(|e| matches!(e, Event::Ask { .. })) else {
        panic!("no question");
    };

    let carried = session.tally();
    let held = carried
        .pending
        .get(id)
        .expect("the question would not survive an upgrade");
    assert_eq!(held.tool, "Bash");
    assert_eq!(held.input["command"], "rm -rf /tmp/x");
    // The CLI's own sentence travels too, or the row comes back reassembled.
    assert_eq!(held.detail.as_deref(), Some("delete a directory"));

    // Answered, and then it is no longer something to carry.
    session.decide(id, true, "", None).await.expect("approve");
    assert!(session.tally().pending.is_empty());
}

#[tokio::test]
async fn a_session_hands_on_the_numbers_it_has_counted() {
    // The other half: what [`handover`] reads off a live session has to be the
    // same numbers its summary shows, or the carrying is of something else.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");

    let summary = session.summary();
    let tally = session.tally();
    assert_eq!(tally.started, summary.started);
    assert_eq!(tally.window, summary.window);
}

#[tokio::test]
async fn nothing_is_inherited_when_this_image_was_not_exec_by_an_upgrade() {
    // The ordinary start. A stray or absent handover must produce a console that
    // simply has no sessions, not one that fails to start.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    assert_eq!(roster.inherit(), 0);
}

#[tokio::test]
async fn a_session_reports_the_mode_it_was_actually_started_with() {
    // ⚠ **Unset is not unknown.** With no `--permission-mode` the CLI runs on
    // its own default, under which every tool call needing permission comes back
    // to whoever is holding the phone. Saying nothing about the mode there
    // leaves the header silent about the one setting that governs every tap —
    // and the first version of this read the mode off the transcript instead,
    // which for a session resumed from an interactive one reported `auto` over a
    // console that was asking permission for everything.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");

    assert_eq!(session.summary().mode.as_deref(), Some("default"));
}

#[tokio::test]
async fn changing_the_mode_is_reported_at_once() {
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");

    session.set_mode("acceptEdits").await.expect("set mode");

    assert_eq!(session.summary().mode.as_deref(), Some("acceptEdits"));
}

#[tokio::test]
async fn a_mode_change_that_could_not_be_sent_leaves_the_old_one_showing() {
    // The header must never claim a mode the session was not told about. The old
    // value is the true one until the new one has at least left the building.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.stop().await;

    assert!(session.set_mode("bypassPermissions").await.is_err());
    assert_eq!(session.summary().mode.as_deref(), Some("default"));
}

#[tokio::test]
async fn a_session_carried_by_an_older_image_is_not_left_blank_about_permissions() {
    // ⚠ A handover written before the mode was carried has no such field, and
    // serde reads that as `None`. Left alone, an upgraded session shows nothing
    // where "what may this do" belongs — which reads as the careful setting, and
    // is the one case it might not be. Seen for real: the first upgrade after
    // this feature landed dropped the mode off the header entirely.
    let tally = console::session::Tally {
        started: 1_754_000_000,
        mode: None,
        ..Default::default()
    };
    let (_stdin_read, stdin) = carried_pipe();
    let (stdout, _stdout_write) = carried_pipe();
    let (stderr, _stderr_write) = carried_pipe();

    let session = console::session::Session::adopt(
        "not-a-session-on-disk".into(),
        std::env::temp_dir(),
        std::process::id(),
        console::session::Fds {
            stdin,
            stdout,
            stderr,
        },
        tally,
    )
    .expect("adopt");

    // `adopt` itself keeps what it was handed; the roster is what fills the gap,
    // because only it knows how this console starts sessions.
    assert_eq!(session.summary().mode, None);
    assert_eq!(console::session::DEFAULT_MODE, "default");
}
