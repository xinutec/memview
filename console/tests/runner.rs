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
        // Into the scratch directory, so a test neither reads nor writes the
        // sentences this machine has paid for.
        gists: dir.join("gists.json"),
        modes: dir.join("modes.json"),
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
async fn work_left_running_is_counted_until_the_harness_says_it_is_done() {
    // ⚠ **The list is drawn without opening anything.** This used to be counted
    // by the session's own page from its event stream, so the one screen that
    // could say "something is still running here" was the screen you had to be
    // on already. The runner watches the same two events and the count rides the
    // summary.
    //
    // A detached call answers at once, saying that it has left something
    // running — which is what the runner counts, rather than the arguments it
    // was called with; see `protocol::detached`. The only signal that the work
    // has finished is the harness's notification, which arrives filed as a user
    // message nobody typed.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;

    assert_eq!(
        session.summary().background,
        0,
        "nothing has been started yet"
    );

    session.send("do it in the background").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::ToolResult { .. }))
    })
    .await;
    assert_eq!(
        session.summary().background,
        1,
        "started and not reported finished"
    );

    session
        .send("tell me when that finished")
        .await
        .expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Background { .. }))
    })
    .await;
    assert_eq!(
        session.summary().background,
        0,
        "the notification names the call that started it, and closes it"
    );
}

#[tokio::test]
async fn killing_one_brings_the_count_down_because_nothing_else_will() {
    // ⚠ **The ending that reports nothing.** A stopped task never produces a
    // notification, and the notification is what the count is keyed by — so the
    // number stayed up for the life of the session. Measured over this machine's
    // transcripts: 162 kills, none of them notified, 162 of the 209 counts that
    // never came down. Seen on this console — a watcher started at 11:03:34 and
    // stopped thirteen seconds later still read as one task running two hours on.
    //
    // The stopping call names the *task*, which is the only name it has to give;
    // the started call is never heard from again. That is why the session holds
    // both names against one entry.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;

    session.send("do it in the background").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::ToolResult { .. }))
    })
    .await;
    assert_eq!(session.summary().background, 1, "started and left running");

    session.send("stop that").await.expect("send");
    until(&session, |seen| {
        seen.iter()
            .filter(|e| matches!(e, Event::ToolResult { .. }))
            .count()
            > 1
    })
    .await;
    assert_eq!(
        session.summary().background,
        0,
        "a kill ends the work as surely as a notification does"
    );
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
        // Into the scratch directory, so a test neither reads nor writes the
        // sentences this machine has paid for.
        gists: root.join("gists.json"),
        modes: root.join("modes.json"),
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
        asked: Some("the first thing it was ever asked".into()),
        cost_usd: 1.25,
        window: Some(1_000_000),
        limit: Some("allowed_warning".into()),
        busy: None,
        pending: Default::default(),
        background: Default::default(),
        spent: Default::default(),
        counted: Default::default(),
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
    // ⚠ **The label, which the transcript DOES record and cannot give back.**
    // A re-seed replays one page, so `asked` bound to whatever prompt started it
    // and moved on every upgrade — see the note on `Tally::asked` (memview
    // #1146).
    assert_eq!(
        summary.asked.as_deref(),
        Some("the first thing it was ever asked"),
        "the session lost its name across the handover"
    );
    assert_eq!(summary.model.as_deref(), Some("claude-opus-5"));
    assert_eq!(summary.window, Some(1_000_000), "no window to be full of");
    assert_eq!(summary.limit.as_deref(), Some("allowed_warning"));
    assert!((summary.cost_usd - 1.25).abs() < f64::EPSILON);
}

#[tokio::test]
async fn an_upgrade_keeps_a_session_that_was_working_working() {
    // ⚠ **The defect this exists for, measured on the phone.** A status is
    // announced when it CHANGES, and none of them is written to the transcript —
    // so a session mid-turn when the console replaced itself came back with
    // nothing saying so, and the re-seed could not put it back. The front page
    // read `idle` over a conversation that was busy compacting, and went on
    // saying so until it next printed something, which was minutes.
    //
    // Both halves in one test: that `tally()` takes the flag off a live session,
    // and that `adopt` puts it back. Building the tally by hand would prove only
    // the second, and it is the first that nothing else covers.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("this will take a while").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Busy { .. }))
    })
    .await;

    let carried = session.tally();
    assert_eq!(
        carried.busy.as_deref(),
        Some("requesting"),
        "what it was doing would not survive an upgrade"
    );

    let (_stdin_read, stdin) = carried_pipe();
    let (stdout, _stdout_write) = carried_pipe();
    let (stderr, _stderr_write) = carried_pipe();
    let after = console::session::Session::adopt(
        // No transcript by this name, so a re-seed cannot account for it.
        "not-a-session-on-disk".into(),
        std::env::temp_dir(),
        std::process::id(),
        console::session::Fds {
            stdin,
            stdout,
            stderr,
        },
        carried,
    )
    .expect("adopt");

    assert_eq!(
        after.summary().busy.as_deref(),
        Some("requesting"),
        "the upgraded console calls a working session idle"
    );
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
        // The call it is asking about, which must survive the upgrade with it —
        // without it the re-seeded question cannot say which tool row it belongs
        // to, and draws a second widget beside it.
        call: Some("toolu_upgraded".into()),
        input: serde_json::json!({"questions": [{"question": "which way"}]}),
        title: None,
        detail: Some("a question standing when the console was replaced".into()),
    };
    let tally = console::session::Tally {
        started: 1_754_000_000,
        model: Some("claude-opus-5".into()),
        mode: Some("auto".into()),
        asked: Some("the first thing it was ever asked".into()),
        cost_usd: 0.0,
        window: None,
        limit: None,
        busy: None,
        pending: std::collections::BTreeMap::from([("ask-1".to_string(), asked)]),
        background: Default::default(),
        spent: Default::default(),
        counted: Default::default(),
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

/// A session driven through real pipes, so what reaches the CLI can be read.
///
/// `adopt` is the only way in that hands the test both ends: the write end of
/// stdout to speak as the CLI, and the read end of stdin to see what it is told.
fn wired() -> (
    Arc<console::session::Session>,
    std::fs::File,
    std::fs::File,
    std::fs::File,
) {
    use std::os::fd::FromRawFd;
    let (stdin_read, stdin) = carried_pipe();
    let (stdout, stdout_write) = carried_pipe();
    let (stderr, stderr_write) = carried_pipe();
    let session = console::session::Session::adopt(
        "not-a-session-on-disk".into(),
        std::env::temp_dir(),
        std::process::id(),
        console::session::Fds {
            stdin,
            stdout,
            stderr,
        },
        Default::default(),
    )
    .expect("adopt");
    // SAFETY: each descriptor came from `pipe(2)` above and is given away once.
    unsafe {
        (
            session,
            std::fs::File::from_raw_fd(stdin_read),
            std::fs::File::from_raw_fd(stdout_write),
            std::fs::File::from_raw_fd(stderr_write),
        )
    }
}

/// What the session has been told, as far as it has been told anything.
///
/// Non-blocking: the point of most of these reads is that NOTHING was written,
/// and a blocking read would hang rather than fail.
fn told(stdin: &std::fs::File) -> String {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    // SAFETY: an fcntl on a descriptor this test owns.
    unsafe { libc::fcntl(stdin.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) };
    let mut said = String::new();
    let mut buffer = [0u8; 4096];
    let mut handle = stdin;
    while let Ok(read) = handle.read(&mut buffer) {
        if read == 0 {
            break;
        }
        said.push_str(&String::from_utf8_lossy(&buffer[..read]));
    }
    said
}

const SAID: &str = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"working on it"}}}"#;
const ENDED: &str =
    r#"{"type":"result","subtype":"success","total_cost_usd":1.0,"num_turns":1,"duration_ms":5}"#;

#[tokio::test]
async fn a_command_sent_mid_turn_waits_for_the_turn_rather_than_becoming_prose() {
    // ⚠ **The defect, measured 2026-08-08 against CLI 2.1.221/226.** A slash
    // command written to a working session is not run: the CLI parks it as a
    // `queued_command` with `commandMode: "prompt"` and hands it to the MODEL as
    // words. `/rename` sent from the phone got "Noted the rename (CLI-side,
    // nothing for me to do)" and no name was ever written, with nothing on
    // screen saying the command had been demoted.
    use std::io::Write;
    let (session, stdin, mut stdout, _stderr) = wired();

    // Working, as this console's sessions usually are.
    writeln!(stdout, "{SAID}").expect("speak");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Text { .. }))
    })
    .await;
    assert!(session.summary().working, "the session is not working");
    let _ = told(&stdin);

    session.send("/compact").await.expect("send");
    assert_eq!(
        told(&stdin),
        "",
        "the command went to the CLI mid-turn, which hands it to the model as words"
    );
    assert_eq!(
        session.summary().held,
        vec!["/compact".to_string()],
        "nothing on screen would say the command is waiting"
    );

    // An ordinary message is NOT held: it is what the queue is for, and the CLI
    // does the right thing with it.
    session.send("and some words").await.expect("send");
    assert!(
        told(&stdin).contains("and some words"),
        "a message was held back with the commands"
    );

    // The turn ends, and only then does the command go.
    writeln!(stdout, "{ENDED}").expect("end the turn");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Command { .. }))
    })
    .await;
    assert!(
        told(&stdin).contains("/compact"),
        "the command never went at all, which is worse than sending it early"
    );
    assert!(
        session.summary().held.is_empty(),
        "the chip would still be on screen over a command that has run"
    );
}

#[tokio::test]
async fn a_held_command_can_be_taken_back() {
    use std::io::Write;
    let (session, stdin, mut stdout, _stderr) = wired();
    writeln!(stdout, "{SAID}").expect("speak");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Text { .. }))
    })
    .await;
    session.send("/compact").await.expect("send");
    let _ = told(&stdin);

    assert!(session.forget_held("/compact"), "it was not holding it");
    assert!(session.summary().held.is_empty());
    // Not an error the second time: two screens can be looking at one session,
    // and the turn can end between the chip being drawn and the tap on it.
    assert!(!session.forget_held("/compact"));

    writeln!(stdout, "{ENDED}").expect("end the turn");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    assert_eq!(
        told(&stdin),
        "",
        "a command that was taken back was sent anyway"
    );
}

#[tokio::test]
async fn a_command_is_not_held_by_a_session_that_is_not_working() {
    // The uncommon case for this console, and the one where the CLI behaves:
    // between turns a command runs as a command, so holding it would add a delay
    // and a chip for nothing.
    let (session, stdin, _stdout, _stderr) = wired();
    assert!(!session.summary().working);
    session.send("/compact").await.expect("send");
    assert!(
        told(&stdin).contains("/compact"),
        "an idle session's command was held instead of run"
    );
    assert!(session.summary().held.is_empty());
}

/// When the console is prepared to say a session has stopped listening.
///
/// ⚠ **This is the alarm that was missing, and its absence cost two manual
/// diagnoses in one morning.** A message written to a session that has gone deaf
/// gets the same *waiting to be read* marker as one a busy session will pick up
/// in a minute, so on 2026-08-08 `hardware` sat silent for twenty minutes twice
/// while the screen said the ordinary thing. See
/// `reference_console_session_stops_reading_stdin`.
mod deafness {
    use console::session::deaf_after;

    /// Ninety seconds, in milliseconds — the threshold, restated here so a test
    /// that disagrees with the constant fails rather than following it.
    const AFTER: i64 = 90_000;
    const NOW: i64 = 1_000_000_000;

    #[test]
    fn nothing_unread_is_never_deaf() {
        // The commonest state there is: a session that finished its turn an hour
        // ago and has been asked nothing since. Silence is not the symptom.
        assert_eq!(
            deaf_after(Some(NOW - 3_600_000), None, None, false, NOW),
            None
        );
    }

    #[test]
    fn a_working_session_is_never_deaf_however_long_it_is_quiet() {
        // ⚠ **The false positive that would have made this useless.** A session
        // ten minutes into a tool call says nothing at all, and parks incoming
        // messages on purpose — measured 2026-08-07, four messages held and
        // released together, the oldest after twelve minutes. `idle_since` is
        // unset while it works, and that is what keeps this quiet.
        assert_eq!(
            deaf_after(None, Some(NOW - 12 * 60_000), None, false, NOW),
            None
        );
    }

    #[test]
    fn the_wait_is_measured_from_whichever_came_second() {
        // A message sent long after the turn ended has not been ignored for as
        // long as the session has been idle — it has been ignored since it was
        // sent, which is the only number that means anything.
        assert_eq!(
            deaf_after(Some(NOW - 3_600_000), Some(NOW - 10_000), None, false, NOW),
            None,
            "sent ten seconds ago, into an hour-old silence"
        );
        // And the other way round: the message was parked mid-turn and the turn
        // has only just ended, so the session has had a second to read it.
        assert_eq!(
            deaf_after(Some(NOW - 1_000), Some(NOW - 3_600_000), None, false, NOW),
            None,
            "the turn ended a second ago"
        );
    }

    #[test]
    fn unread_between_turns_for_long_enough_is_deaf() {
        let idle = Some(NOW - AFTER - 1);
        assert_eq!(
            deaf_after(idle, Some(NOW - AFTER - 1), None, false, NOW),
            Some(AFTER + 1)
        );
        assert_eq!(
            deaf_after(idle, Some(NOW - AFTER + 1), None, false, NOW),
            None
        );
    }

    #[test]
    fn an_answer_the_session_never_acted_on_is_deafness_on_its_own() {
        // ⚠ **The case the message test cannot see, and it cost thirty-one
        // minutes.** A session blocked on a question is MID-TURN, so
        // `idle_since` is unset and the test above is silent for ever. But a
        // session that asked a question and stopped is not working — it said so —
        // and the console had written the answer into its pipe. `health`,
        // 2026-08-08: answered at 09:30:44, still blocked at 10:01, and the card
        // on the phone was green (memview #122).
        assert_eq!(
            deaf_after(None, None, Some(NOW - AFTER - 1), false, NOW),
            Some(AFTER + 1),
            "no turn has ended and nothing is unread — the decision alone is enough"
        );
        assert_eq!(
            deaf_after(None, None, Some(NOW - AFTER + 1), false, NOW),
            None,
            "a decision written a moment ago is an ordinary wait"
        );
    }

    #[test]
    fn the_longer_of_the_two_waits_is_the_one_reported() {
        // Two ways to be waiting, and a short one starting later must not hide a
        // long one already running.
        assert_eq!(
            deaf_after(
                Some(NOW - 40 * 60_000),
                Some(NOW - 40 * 60_000),
                Some(NOW - 60_000),
                false,
                NOW
            ),
            Some(40 * 60_000)
        );
    }

    #[test]
    fn a_compaction_is_given_far_longer_but_not_for_ever() {
        // ⚠ **The one legitimate silence with no pulse.** A compaction leaves
        // the transcript frozen for minutes — measured on `hardware`, sent at
        // 09:50:46 with the file still stopped at 09:49:53 twenty seconds
        // later — so nothing shorter than this can tell it from a fault.
        let waited = |ms: i64| deaf_after(Some(NOW - ms), Some(NOW - ms), None, true, NOW);
        assert_eq!(
            waited(5 * 60_000),
            None,
            "five minutes in, still summarising"
        );
        assert_eq!(
            waited(20 * 60_000),
            Some(20 * 60_000),
            "a session can go deaf around a compaction too, and one did"
        );
    }
}

#[tokio::test]
async fn a_message_stops_being_in_flight_when_the_session_reads_it_back() {
    // The pair the whole alarm rests on: the write says the bytes reached the
    // pipe, and the CLI's replay is the only thing that says they were taken out
    // of it. Nothing else on the wire mentions the trip.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;

    session.send("do the thing").await.expect("send");
    assert_eq!(
        session.unread(),
        vec!["do the thing".to_string()],
        "in flight the moment it is written"
    );
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    assert!(
        session.unread().is_empty(),
        "the replay is the read receipt: {:?}",
        session.unread()
    );
}

#[tokio::test]
async fn a_slash_command_is_never_counted_as_in_flight() {
    // ⚠ **Measured against CLI 2.1.221:** `--replay-user-messages` does not
    // replay a command. Counting one would leave it in flight for ever, and
    // ninety seconds later the console would call a perfectly well session deaf
    // every time anybody typed `/compact`.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;

    session.send("/compact").await.expect("send");
    assert!(session.unread().is_empty(), "{:?}", session.unread());
    until(&session, |seen| {
        seen.iter()
            .any(|e| matches!(e, Event::Command { text } if text == "/compact"))
    })
    .await;
}

/// What a conversation is allowed to do, across everything that forgets.
///
/// ⚠ **Resuming used to drop a session to Manual and report that as the truth.**
/// Measured 2026-08-08: `hardware` was in `auto`, was stopped and resumed, and
/// came back `default` — a session that then stops at the first tool call
/// needing approval and waits, which from a phone is the stall it was restarted
/// for (memview #119).
mod remembering_the_mode {
    use super::*;
    use console::modes::Modes;

    #[test]
    fn a_mode_survives_the_console_that_learnt_it() {
        // ⚠ **The case that actually happened, and the reason memory is not
        // enough.** An upgrade carries only LIVE sessions, so the ended one is
        // dropped from the roster — and an ended session is precisely what
        // somebody is resuming. Nothing in the process knows the mode by then;
        // only the file does.
        let dir = std::env::temp_dir().join(format!("modes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        let store = dir.join("modes.json");
        let _ = std::fs::remove_file(&store);

        let learnt = Modes::load(store.clone());
        learnt.set("hardware", "auto");
        drop(learnt);

        assert_eq!(
            Modes::load(store).get("hardware").as_deref(),
            Some("auto"),
            "a console that has restarted still knows"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_conversation_nobody_has_spoken_for_claims_nothing() {
        // Absent, not `default`. The two are different: one is "it was left on
        // Manual" and the other is "nobody knows", and only the second may be
        // overridden by the console's own configuration.
        let store = std::env::temp_dir().join("modes-never-written-at-all.json");
        let _ = std::fs::remove_file(&store);
        assert_eq!(Modes::load(store).get("whoever"), None);
    }

    #[tokio::test]
    async fn a_resumed_session_comes_back_on_the_mode_it_was_left_in() {
        let dir = std::env::temp_dir();
        let roster = roster(&dir);
        // Left deliberately on something other than the console's default, which
        // is what makes losing it cost anything.
        roster.remember_mode("a-conversation-from-before", "acceptEdits");

        let session = roster
            .resume(&dir.display().to_string(), "a-conversation-from-before")
            .expect("resume");
        assert_eq!(
            session.summary().mode.as_deref(),
            Some("acceptEdits"),
            "not the console's configured mode, and not Manual"
        );
    }
}

/// Whether a turn is running, as the runner observes it.
///
/// ⚠ **The console called a working session idle.** Reported from the phone
/// 2026-08-07: "It says you're idle. My messages aren't seen by you yet." The
/// session was mid-turn throughout, running tools — but `busy` is announced only
/// when it CHANGES, and no status was drawn as *idle* (memview #112).
mod whether_it_is_working {
    use super::*;

    #[tokio::test]
    async fn a_session_that_has_not_said_anything_yet_is_not_working() {
        // ⚠ **The case that shipped wrong.** `working` was derived from "no turn
        // has ended", which is also true of a session that has said NOTHING —
        // still starting, no `Started` line on the wire. A brand-new session
        // with an empty transcript was reported as working, on screen, within a
        // minute of shipping it.
        let dir = std::env::temp_dir();
        let roster = roster(&dir);
        let session = roster.start(&dir.display().to_string()).expect("start");
        assert!(
            !session.summary().working,
            "nothing has been heard from it at all"
        );
    }

    #[tokio::test]
    async fn a_fresh_session_with_nothing_to_do_is_not_working() {
        let dir = std::env::temp_dir();
        let roster = roster(&dir);
        let session = roster.start(&dir.display().to_string()).expect("start");
        until(&session, |seen| {
            seen.iter().any(|e| matches!(e, Event::Started { .. }))
        })
        .await;
        assert!(
            !session.summary().working,
            "it has never had a turn to end, which is not the same as being in one"
        );
    }

    #[tokio::test]
    async fn a_session_is_working_from_the_first_thing_it_says_until_the_turn_ends() {
        let dir = std::env::temp_dir();
        let roster = roster(&dir);
        let session = roster.start(&dir.display().to_string()).expect("start");
        until(&session, |seen| {
            seen.iter().any(|e| matches!(e, Event::Started { .. }))
        })
        .await;

        session.send("do the thing").await.expect("send");
        // The stub answers in one go, so catching the middle of a turn reliably
        // is not what this can test. What it CAN pin is the end state, which is
        // the half that was wrong: after the result line, not working.
        until(&session, |seen| {
            seen.iter().any(|e| matches!(e, Event::Turn { .. }))
        })
        .await;
        assert!(!session.summary().working, "the turn ended");
    }

    /// ⚠ **84 minutes of `working` over a process doing nothing.** `hardware`
    /// was resumed 2026-08-08 22:53; its transcript ended mid-turn that morning,
    /// so the seeded events said "speaking" and no `Turn` ever followed to take
    /// it back. The card claimed a turn was running while the process held no
    /// API socket, a flat 0.5% of a core and a static 709 MB — and a message
    /// sent at 00:17 was picked up at once, because nothing was ever wrong with
    /// it. `Joined` set `idle_since` and left `working` alone, so the session
    /// was marked idle and mid-turn at the same time (memview #640).
    ///
    /// Against the rule rather than a spawned session: reaching this needs a
    /// transcript that ends mid-turn and a resume to read it, which is the
    /// machinery that kept it untested.
    mod what_a_resume_carries {
        use console::protocol::Event;
        use console::session::working_after;

        fn spoke() -> Event {
            Event::Text {
                text: "half a sentence".into(),
            }
        }

        #[test]
        fn joining_a_transcript_that_ends_mid_turn_does_not_inherit_its_turn() {
            let mid_turn = working_after(false, &spoke());
            assert!(mid_turn, "the seeded transcript's last word");
            assert!(
                !working_after(
                    mid_turn,
                    &Event::Joined {
                        earlier: 12,
                        from: 0,
                        restarted: true
                    }
                ),
                "a process that has only just started is not in that turn"
            );
        }

        #[test]
        fn a_fresh_spawn_is_not_working_either() {
            assert!(!working_after(
                true,
                &Event::Started {
                    model: "claude-opus-5".into(),
                    cwd: "/home/example".into(),
                    tools: 3,
                }
            ));
        }

        #[test]
        fn anything_that_says_nothing_either_way_leaves_the_answer_alone() {
            // A status, notably: it is announced only when it CHANGES, so its
            // presence can be minutes old and its absence says nothing at all.
            let quiet = Event::Answered {
                id: "1".into(),
                allowed: true,
                reply: None,
            };
            assert!(working_after(true, &quiet));
            assert!(!working_after(false, &quiet));
        }
    }
}

#[tokio::test]
async fn a_stop_writes_down_when_the_kill_is_due() {
    // ⚠ **Because the timer that carries it does not survive an upgrade.** The
    // kill lives in a `tokio::spawn`, and `handover` re-execs this process; the
    // session is not carried either, since closing stdin makes its descriptors
    // unkeepable. A stopped session ran on for two and a quarter hours,
    // owned by nobody. The deadline is written down so the next image can
    // finish what this one started — see #750.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;
    assert_eq!(
        session.stopping(),
        None,
        "a session nobody has stopped has no kill owed to it"
    );

    let at = console::session::now();
    session.stop().await;

    let due = session.stopping().expect("a stop arms a kill");
    // The grace itself, give or take the time this test took: generous on
    // purpose, because the process may be mid-tool-call and the clean exit is
    // the one that flushes the transcript.
    assert!(
        due >= at + 25_000 && due <= at + 35_000,
        "the deadline is the grace period away, not {}ms",
        due - at
    );
}

#[test]
fn a_listing_that_merely_mentions_a_conversation_is_not_that_conversation() {
    // ⚠ The trap `words_of_claude_processes` exists for, reached from the other
    // side: this decides whether a SIGKILL is sent, so a line matching by
    // accident is a kill aimed at whatever that pid happens to be now.
    let id = "b1b1b1b1-0000-4000-8000-000000000003";
    assert!(console::session::names_session(
        &format!("/usr/local/bin/claude -p --resume {id} --permission-mode auto"),
        id
    ));
    assert!(
        !console::session::names_session(&format!("/bin/grep -n {id} console.log"), id),
        "a grep for the id is not the session"
    );
    // What ps prints for a pid that has gone: nothing at all.
    assert!(!console::session::names_session("", id));
    assert!(
        !console::session::names_session(
            "/usr/local/bin/claude -p --resume 11111111-1111-4111-8111-111111111111",
            id
        ),
        "another conversation's process is not this one"
    );
}

#[test]
fn a_pid_that_is_not_that_conversation_any_more_is_left_alone() {
    // ⚠ **The whole risk in finishing a stop late.** The deadline is up to
    // thirty seconds old and a pid is not a handle: by the time it comes round,
    // the system may have started something else in that pid's place. The
    // warning is already written down at `Roster::revive`; this is the guard.
    //
    // Stood up as a real process rather than a made-up pid, because the failure
    // being guarded against is a kill that lands, and only a live process can
    // show that it did not.
    let mut other = std::process::Command::new("sleep")
        .arg("30")
        .spawn()
        .expect("a process to not kill");
    let pid = other.id();

    console::session::finish(pid, "b1b1b1b1-0000-4000-8000-000000000003");

    assert!(
        other.try_wait().expect("wait").is_none(),
        "it killed a process that was never that session"
    );
    let _ = other.kill();
    let _ = other.wait();
}

/// A process whose command line looks like a `claude` running `id`, so that the
/// guard in `finish` lets a kill through to it.
///
/// ⚠ **A symlink named `claude`, because that is exactly what the guard reads.**
/// `words_of_claude_processes` decides on the first word's last path element —
/// the account of why is beside it — so a test that used a differently-named
/// process would only ever exercise the refusing half.
fn wearing_the_name(dir: &std::path::Path, id: &str) -> Disguised {
    std::fs::create_dir_all(dir).expect("dir");
    let looks_like = dir.join("claude");
    let _ = std::fs::remove_file(&looks_like);
    std::os::unix::fs::symlink("/bin/sh", &looks_like).expect("symlink");
    // ⚠ **A shell rather than `sleep` itself, and the loop is not decoration.**
    // `sleep 30 <id>` looks right and exits immediately — "invalid time
    // interval" — so both of these tests passed while proving nothing, which an
    // ablation of the kill caught by staying green. And a shell given one plain
    // command execs it, replacing its own argv and taking the id off the command
    // line the guard reads; a loop is a command it has to stay alive to run.
    let child = std::process::Command::new(&looks_like)
        .args(["-c", "while :; do sleep 1; done", id])
        .spawn()
        .expect("a process wearing the name");
    Disguised {
        child,
        dir: dir.to_path_buf(),
    }
}

/// The disguised process, cleaned up however the test ends.
///
/// ⚠ **`Drop`, and not a line at the end of the test.** A failed assertion
/// unwinds straight past that line, and what it leaves behind is a shell in a
/// spin loop still holding the test binary's stdout — which is a run that never
/// finishes rather than one that fails. Measured while ablating the kill on
/// purpose to check these tests were worth anything: ten minutes, no result.
struct Disguised {
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Disguised {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether it has gone, asked once.
    fn gone(&mut self) -> bool {
        self.child.try_wait().expect("wait").is_some()
    }

    /// Whether it has gone, waiting up to five seconds for it to.
    ///
    /// SIGKILL is delivered by the time `kill` returns, but the process table
    /// takes a moment to catch up, so this waits rather than asking once.
    ///
    /// ⚠ **Blocking, so it is for the synchronous test only.** A `tokio::test`
    /// is a current-thread runtime: sleeping the thread inside one starves the
    /// task the code under test just spawned, and the kill that never arrives
    /// looks exactly like the bug. Seen here — the async test below failed
    /// against a working fix until it awaited instead.
    fn died(&mut self) -> bool {
        let since = std::time::Instant::now();
        while since.elapsed().as_secs() < 5 {
            if self.gone() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }
}

impl Drop for Disguised {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn the_kill_does_land_on_the_process_that_is_still_that_conversation() {
    // The other half of the guard, and the half that matters: refusing every
    // kill would also pass the test above while leaving #750 exactly as it was.
    let id = "b1b1b1b1-0000-4000-8000-000000000001";
    let dir = std::env::temp_dir().join(format!("console-finish-{}", std::process::id()));
    let mut wearing = wearing_the_name(&dir, id);

    console::session::finish(wearing.pid(), id);

    assert!(wearing.died(), "the process outlived the kill it was owed");
}

#[tokio::test]
async fn a_kill_the_last_image_could_not_deliver_is_delivered_by_this_one() {
    // ⚠ **The whole of #750, from the receiving end.** The old image wrote down
    // what it was in the middle of stopping; this reads it and finishes the job
    // it could not, because `execve` took its timer with it.
    //
    // The environment is process-wide, so this is the only test that touches
    // that variable — and it is removed on read, which is itself the thing being
    // relied on: a variable left behind would have the NEXT upgrade aim a kill
    // at a pid that has already been dealt with.
    let id = "b1b1b1b1-0000-4000-8000-000000000002";
    let dir = std::env::temp_dir().join(format!("console-owed-{}", std::process::id()));
    let mut wearing = wearing_the_name(&dir, id);
    // Due now: the deadline travels as an absolute moment, so one that has
    // already passed means "there is nothing left of the grace", not "wait".
    let owed =
        serde_json::json!([{ "id": id, "pid": wearing.pid(), "due": console::session::now() }]);
    unsafe { std::env::set_var("CONSOLE_HANDOVER_STOPPING", owed.to_string()) };

    let roster = roster(&std::env::temp_dir());
    assert_eq!(roster.finish_stopping(), 1, "it read what it was handed");
    assert!(
        std::env::var("CONSOLE_HANDOVER_STOPPING").is_err(),
        "the variable outlived the read, so the next upgrade would fire it again"
    );

    // Awaited rather than slept through: the kill runs on a task this very
    // runtime has to be free to poll. See [`Disguised::died`].
    let since = std::time::Instant::now();
    while !wearing.gone() && since.elapsed().as_secs() < 5 {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        wearing.gone(),
        "the stop was owed a kill and this image did not deliver it either"
    );
}
