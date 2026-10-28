//! A message sent under the phone's id, over the wire, to a session: delivered
//! once however often it is retried, and taken back only while the session has
//! not read it — a lost receipt, then a withdrawal, end to end.
//!
//! Against the stand-in CLI, which records one message per id as the real one
//! does (`tests/fixtures/stub-cli`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use console::api;
use console::config::Config;
use console::protocol::Event;
use console::roster::Roster;
use console::session::{Session, Spawn};

fn roster(dir: &std::path::Path) -> Arc<Roster> {
    Arc::new(Roster::new(Config {
        bind: "127.0.0.1:0".to_string(),
        desk: "127.0.0.1:0".to_string(),
        tls: None,
        dirs: vec![dir.to_path_buf()],
        spawn: Spawn {
            binary: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/stub-cli")
                .display()
                .to_string(),
            model: None,
            name: None,
            permission_mode: None,
        },
        static_dir: None,
        usage_url: None,
        gists: dir.join("gists.json"),
        modes: dir.join("modes.json"),
        drafts: dir.join("drafts.json"),
        tunnel: None,
    }))
}

/// A session behind the API on a port the OS picks, in a directory of its own so
/// no other test's record of sent ids is read.
async fn served() -> (Arc<Session>, String) {
    let dir = std::env::temp_dir().join(format!("outbox-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("dir");
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, api::router(roster)).await;
    });
    let base = format!("http://{addr}/api/sessions/{}", session.id);
    (session, base)
}

async fn post(url: String, body: serde_json::Value) -> (u16, String) {
    let response = reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .expect("request");
    (
        response.status().as_u16(),
        response.text().await.expect("body"),
    )
}

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The messages the session read, with their ids.
fn read(session: &Session) -> Vec<(String, Option<String>)> {
    session
        .history()
        .into_iter()
        .filter_map(|event| match event {
            Event::Prompt { text, id } => Some((text, id)),
            _ => None,
        })
        .collect()
}

fn turns(session: &Session) -> usize {
    session
        .history()
        .iter()
        .filter(|event| matches!(event, Event::Turn { .. }))
        .count()
}

async fn until(session: &Session, what: impl Fn(&Session) -> bool) {
    for _ in 0..100 {
        if what(session) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("waited 5s; it read {:?}", read(session));
}

/// Long enough for a second reply to arrive, were there going to be one.
async fn settled() {
    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[tokio::test]
async fn a_retry_after_a_lost_receipt_reaches_the_session_once() {
    let (session, base) = served().await;
    let m42 = id();
    let message = serde_json::json!({"text": "hello", "id": m42});
    assert_eq!(post(format!("{base}/input"), message.clone()).await.0, 200);
    until(&session, |s| turns(s) == 1).await;

    // The answer was lost on the way back; the phone tries again, twice.
    assert_eq!(post(format!("{base}/input"), message.clone()).await.0, 200);
    assert_eq!(post(format!("{base}/input"), message).await.0, 200);
    settled().await;
    assert_eq!(
        read(&session),
        vec![("hello".to_string(), Some(m42.clone()))]
    );
    assert_eq!(turns(&session), 1, "answered once");
    // And not written again: the CLI would ignore a second copy, but this side
    // never relies on that for an id it already knows.
    let again = session
        .offer(
            &m42,
            "hello",
            Vec::new(),
            console::protocol::prompt("hello", Some(&m42)),
        )
        .await
        .expect("offer");
    assert_eq!(again, console::session::Offered::Already);
}

#[tokio::test]
async fn a_message_the_session_read_cannot_be_taken_back_and_says_when() {
    let (session, base) = served().await;
    let m42 = id();
    post(
        format!("{base}/input"),
        serde_json::json!({"text": "hello", "id": m42}),
    )
    .await;
    until(&session, |s| turns(s) == 1).await;

    let (status, body) = post(format!("{base}/withdraw"), serde_json::json!({"id": m42})).await;
    assert_eq!(status, 409);
    let refusal: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(refusal["why"], "read");
    assert!(refusal["at"].is_i64(), "says when it was read: {body}");
}

#[tokio::test]
async fn a_queued_message_is_taken_back_from_the_cli_and_never_read() {
    // Sent to a busy session, it waits in the CLI's queue; taken back there, it is
    // never said, every client is told, and a late retry is refused.
    let (session, base) = served().await;
    let m42 = id();
    let message = serde_json::json!({"text": "queued: rename the branch", "id": m42});
    assert_eq!(post(format!("{base}/input"), message.clone()).await.0, 200);

    let (status, body) = post(format!("{base}/withdraw"), serde_json::json!({"id": m42})).await;
    assert_eq!(status, 200, "{body}");
    let returned: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(
        returned["text"], "queued: rename the branch",
        "its words come back"
    );
    assert!(
        session.summary().messages.is_empty(),
        "every client is shown it gone"
    );
    // Once: a second tap, or a second device, is told it is already back.
    let (status, body) = post(format!("{base}/withdraw"), serde_json::json!({"id": m42})).await;
    assert_eq!(status, 409);
    assert!(body.contains("\"withdrawn\""), "{body}");
    let (status, body) = post(format!("{base}/input"), message).await;
    assert_eq!(status, 409);
    assert!(body.contains("\"withdrawn\""), "{body}");
    settled().await;
    assert!(read(&session).is_empty());
    assert_eq!(turns(&session), 0);
}

#[tokio::test]
async fn a_take_back_that_reaches_the_cli_before_its_message_still_takes_it_back() {
    // The CLI had not read the message off its input when the cancel came, so it
    // answered `cancelled: false` — and then took the message out the moment it
    // arrived. Read as "the session read it", the phone waited for ever for an
    // echo that never came (measured on 2.1.289).
    let (session, base) = served().await;
    let m42 = id();
    let message = serde_json::json!({"text": "late: rename the branch", "id": m42});
    assert_eq!(post(format!("{base}/input"), message).await.0, 200);

    let (status, body) = post(format!("{base}/withdraw"), serde_json::json!({"id": m42})).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("late: rename the branch"), "{body}");
    settled().await;
    assert!(read(&session).is_empty());
}

#[tokio::test]
async fn a_message_read_mid_turn_is_known_read_by_its_id() {
    // Delivered into a running turn it has no echo: the CLI records it as an
    // attachment carrying the client's id. Without that id the phone showed it
    // "waiting to be read" for ever, and the Mac could not say it was read.
    let (session, base) = served().await;
    let m42 = id();
    post(
        format!("{base}/input"),
        serde_json::json!({"text": "fold: check the build", "id": m42}),
    )
    .await;
    until(&session, |s| turns(s) == 1).await;
    assert_eq!(
        read(&session),
        vec![("fold: check the build".to_string(), Some(m42.clone()))]
    );

    let (status, body) = post(format!("{base}/withdraw"), serde_json::json!({"id": m42})).await;
    assert_eq!(status, 409);
    assert!(body.contains("\"read\""), "{body}");
}

#[tokio::test]
async fn a_message_that_never_arrived_is_taken_back_and_stays_back() {
    let (session, base) = served().await;
    let m42 = id();
    assert_eq!(
        post(format!("{base}/withdraw"), serde_json::json!({"id": m42}))
            .await
            .0,
        200
    );
    let (status, body) = post(
        format!("{base}/input"),
        serde_json::json!({"text": "hello", "id": m42}),
    )
    .await;
    assert_eq!(status, 409);
    assert!(body.contains("\"withdrawn\""), "{body}");
    settled().await;
    assert!(read(&session).is_empty());
}

#[tokio::test]
async fn messages_without_an_id_are_sent_as_before() {
    let (session, base) = served().await;
    post(format!("{base}/input"), serde_json::json!({"text": "one"})).await;
    until(&session, |s| turns(s) == 1).await;
    post(format!("{base}/input"), serde_json::json!({"text": "two"})).await;
    until(&session, |s| turns(s) == 2).await;
    let words: Vec<String> = read(&session).into_iter().map(|(text, _)| text).collect();
    assert_eq!(words, ["one", "two"]);
}

#[tokio::test]
async fn an_id_that_is_not_a_uuid_is_refused() {
    // The CLI takes only a uuid; anything else would fail there, unexplained.
    let (_, base) = served().await;
    let (status, _) = post(
        format!("{base}/input"),
        serde_json::json!({"text": "hi", "id": "m-42"}),
    )
    .await;
    assert_eq!(status, 400);
}
