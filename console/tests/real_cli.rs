//! The message record against the real Claude CLI, not the stand-in: what the
//! stand-in does is only what we believe the CLI does, and twice that belief
//! was wrong. A few Haiku calls a run, so run by hand, and before trusting a CLI
//! upgrade:
//!
//!     CONSOLE_REAL_CLI=1 cargo test -p console --test real_cli -- --test-threads=1
//!
//! Without the variable every test returns at once.

use std::sync::Arc;
use std::time::Duration;

use console::config::Config;
use console::protocol::Event;
use console::roster::Roster;
use console::session::{Offered, Session, Spawn, Withdrawal};

fn real() -> bool {
    std::env::var("CONSOLE_REAL_CLI").is_ok_and(|it| it == "1")
}

fn roster() -> (Arc<Roster>, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("real-cli-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let roster = Arc::new(Roster::new(Config {
        bind: "127.0.0.1:0".to_string(),
        desk: "127.0.0.1:0".to_string(),
        tls: None,
        dirs: vec![dir.clone()],
        spawn: Spawn {
            binary: "claude".to_string(),
            model: Some("claude-haiku-4-5-20251001".to_string()),
            name: None,
            permission_mode: None,
        },
        static_dir: None,
        usage_url: None,
        gists: dir.join("gists.json"),
        modes: dir.join("modes.json"),
        drafts: dir.join("drafts.json"),
        reach: dir.join("unreached.json"),
        tunnel: None,
    }));
    (roster, dir)
}

async fn until(session: &Session, what: impl Fn(&Session) -> bool, seconds: u64) {
    for _ in 0..seconds * 10 {
        if what(session) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("waited {seconds}s: {:?}", session.messages());
}

fn turns(session: &Session) -> usize {
    session
        .history()
        .iter()
        .filter(|event| matches!(event, Event::Turn { .. }))
        .count()
}

fn said(session: &Session, words: &str) -> bool {
    session
        .history()
        .iter()
        .any(|event| matches!(event, Event::Text { text } if text.to_lowercase().contains(words)))
}

async fn offer(session: &Session, id: &str, text: &str) -> Offered {
    let line = console::protocol::prompt(text, Some(id));
    session
        .offer(id, text, Vec::new(), line)
        .await
        .expect("offer")
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[tokio::test]
async fn a_queued_message_is_taken_back_and_never_read() {
    if !real() {
        return;
    }
    let (roster, dir) = roster();
    let session = roster.start(&dir.display().to_string()).expect("start");
    session
        .send("Write a 300 word story about a lighthouse keeper.")
        .await
        .expect("send");
    let queued = id();
    assert_eq!(
        offer(&session, &queued, "Then say the word banana.").await,
        Offered::Wrote
    );
    let back = session.withdraw(&queued).await.expect("withdraw");
    assert!(
        matches!(&back, Withdrawal::Back(Some(message)) if message.text == "Then say the word banana."),
        "{back:?}"
    );
    until(&session, |s| turns(s) >= 1, 60).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(turns(&session), 1, "the message taken back was answered");
    assert!(!said(&session, "banana"));
    assert!(session.messages().is_empty());
    session.force();
}

#[tokio::test]
async fn a_message_read_is_refused_and_a_retry_is_not_sent_again() {
    if !real() {
        return;
    }
    let (roster, dir) = roster();
    let session = roster.start(&dir.display().to_string()).expect("start");
    let message = id();
    assert_eq!(
        offer(&session, &message, "Reply with just the word ok.").await,
        Offered::Wrote
    );
    until(&session, |s| turns(s) >= 1, 60).await;
    assert!(session.messages().is_empty(), "read, and no longer waiting");

    assert_eq!(
        offer(&session, &message, "Reply with just the word ok.").await,
        Offered::Already
    );
    assert!(matches!(
        session.withdraw(&message).await.expect("withdraw"),
        Withdrawal::Refused(console::sent::Refusal {
            why: console::sent::Why::Read,
            ..
        })
    ));
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(turns(&session), 1);
    session.force();
}

#[tokio::test]
async fn a_held_command_is_taken_back_and_never_run() {
    if !real() {
        return;
    }
    let (roster, dir) = roster();
    let session = roster.start(&dir.display().to_string()).expect("start");
    session
        .send("Write a 300 word story about a baker.")
        .await
        .expect("send");
    until(&session, |s| s.summary().working, 60).await;
    let held = id();
    assert_eq!(offer(&session, &held, "/compact").await, Offered::Wrote);
    assert_eq!(
        session.messages().first().map(|it| it.stands),
        Some(console::session::Stands::Held)
    );
    assert!(matches!(
        session.withdraw(&held).await.expect("withdraw"),
        Withdrawal::Back(Some(_))
    ));
    until(&session, |s| turns(s) >= 1, 60).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !session
            .history()
            .iter()
            .any(|event| matches!(event, Event::Command { text } if text == "/compact")),
        "the command taken back ran"
    );
    session.force();
}

#[tokio::test]
async fn a_message_waiting_when_the_process_is_killed_is_dropped() {
    if !real() {
        return;
    }
    let (roster, dir) = roster();
    let session = roster.start(&dir.display().to_string()).expect("start");
    session
        .send("Write a 300 word story about a gardener.")
        .await
        .expect("send");
    let waiting = id();
    offer(&session, &waiting, "Then say the word banana.").await;
    session.force();
    until(&session, |s| !s.alive(), 20).await;
    assert_eq!(
        session.messages().first().map(|it| it.stands),
        Some(console::session::Stands::Dropped)
    );
}
