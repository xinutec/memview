//! A message sent under the phone's id, over the wire, to a session: delivered
//! once however often it is retried, and edited or taken back only while it can
//! be — a lost receipt, then a cancel or an edit, end to end.
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
        sent: dir.join("sent.json"),
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
    // And written once: the CLI would ignore a second copy, but this side never
    // relies on that for an id it already knows.
    let written = session
        .history()
        .iter()
        .filter(|event| matches!(event, Event::Accepted { id: Some(id), .. } if *id == m42))
        .count();
    assert_eq!(written, 1);
}

#[tokio::test]
async fn cancelling_a_message_that_got_there_is_refused_with_when() {
    let (session, base) = served().await;
    let m42 = id();
    post(
        format!("{base}/input"),
        serde_json::json!({"text": "hello", "id": m42}),
    )
    .await;
    until(&session, |s| turns(s) == 1).await;

    let (status, body) = post(format!("{base}/cancel"), serde_json::json!({"id": m42})).await;
    assert_eq!(status, 409);
    let refusal: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(refusal["why"], "delivered");
    assert!(refusal["at"].is_i64(), "says when it got there: {body}");
}

#[tokio::test]
async fn an_edit_of_a_message_that_got_there_is_not_sent() {
    // Sent, delivered, receipt lost; then cancelled, edited and sent again. The
    // session must not get both.
    let (session, base) = served().await;
    let m42 = id();
    post(
        format!("{base}/input"),
        serde_json::json!({"text": "hello", "id": m42}),
    )
    .await;
    until(&session, |s| turns(s) == 1).await;

    let m43 = id();
    let (status, body) = post(
        format!("{base}/input"),
        serde_json::json!({"text": "hello, edited", "id": m43, "replaces": m42}),
    )
    .await;
    assert_eq!(status, 409);
    let refusal: serde_json::Value = serde_json::from_str(&body).expect("json");
    assert_eq!(refusal["why"], "delivered");
    assert_eq!(
        refusal["original"], true,
        "about the original, not the edit"
    );
    settled().await;
    assert_eq!(read(&session), vec![("hello".to_string(), Some(m42))]);
}

#[tokio::test]
async fn an_edit_of_a_message_that_never_arrived_replaces_it() {
    let (session, base) = served().await;
    let m42 = id();
    let m43 = id();
    let (status, _) = post(
        format!("{base}/input"),
        serde_json::json!({"text": "hello, edited", "id": m43, "replaces": m42}),
    )
    .await;
    assert_eq!(status, 200);
    until(&session, |s| turns(s) == 1).await;

    // The original's retry, still somewhere in the network, arrives late.
    let (status, body) = post(
        format!("{base}/input"),
        serde_json::json!({"text": "hello", "id": m42}),
    )
    .await;
    assert_eq!(status, 409);
    assert!(body.contains("\"cancelled\""), "{body}");
    settled().await;
    assert_eq!(
        read(&session),
        vec![("hello, edited".to_string(), Some(m43))]
    );
}

#[tokio::test]
async fn a_retry_of_an_edit_is_answered_without_a_second_message() {
    let (session, base) = served().await;
    let (m42, m43) = (id(), id());
    let edit = serde_json::json!({"text": "hello, edited", "id": m43, "replaces": m42});
    assert_eq!(post(format!("{base}/input"), edit.clone()).await.0, 200);
    until(&session, |s| turns(s) == 1).await;
    assert_eq!(post(format!("{base}/input"), edit).await.0, 200);
    settled().await;
    assert_eq!(read(&session).len(), 1);
}

#[tokio::test]
async fn a_cancelled_message_stays_cancelled() {
    let (session, base) = served().await;
    let m42 = id();
    assert_eq!(
        post(format!("{base}/cancel"), serde_json::json!({"id": m42}))
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
    assert!(body.contains("\"cancelled\""), "{body}");
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
