//! The work a conversation is holding, read from the tasks service.
//!
//! Against a stub service rather than the live one: a suite that needed isis up
//! would fail on a train and pass at a desk, and would be measuring the tunnel
//! rather than the reader. The shapes here are the ones that break a naive
//! client — a session holding nothing, a service that will not answer, and a
//! second read that must not become a second request.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use console::tasks::Tasks;

/// A stub service: answers every path from a table, and counts what it was
/// asked. Returns its address and the counter, and stops when the test ends.
async fn serving(answers: Vec<(&'static str, &'static str)>) -> (SocketAddr, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("addr");
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = asked.clone();
    let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
        let answers = answers.clone();
        let counted = counted.clone();
        async move {
            counted.fetch_add(1, Ordering::SeqCst);
            let path = uri.path().to_string();
            answers
                .iter()
                .find(|(at, _)| *at == path)
                .map(|(_, body)| {
                    ([(axum::http::header::CONTENT_TYPE, "application/json")], *body)
                        .into_response()
                })
                .unwrap_or_else(|| axum::http::StatusCode::NOT_FOUND.into_response())
        }
    });
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (address, asked)
}

use axum::response::IntoResponse;

/// A reader pointed at `address`.
///
/// ⚠ **Told where to look, rather than through `TASKS_URL`.** These tests run in
/// parallel in one process, and an environment variable is shared by all of
/// them: setting it per test had each stub answering somebody else's reader.
fn reading(address: SocketAddr) -> Tasks {
    // ⚠ **The provider, which `main.rs` installs and a test harness does not.**
    // The console builds reqwest against `rustls-…-no-provider`, so a client
    // built without this panics inside the builder with `No provider set` —
    // before any request, and identically for every test here. Idempotent, so
    // every test may call it.
    let _ = rustls::crypto::ring::default_provider().install_default();
    Tasks::at(format!("http://{address}"))
}

#[tokio::test]
async fn a_session_holding_nothing_is_absent_rather_than_zero() {
    // The rule the client draws by: no work is not an empty list, and a card
    // saying `0` would be a claim where nothing is the truth.
    let body = r#"[{"id":"alive","open":3,"first_seen":"2026-08-08T10:00:00Z","last_seen":"2026-08-08T10:00:00Z"},
                   {"id":"idle","open":0,"first_seen":"2026-08-08T10:00:00Z","last_seen":"2026-08-08T10:00:00Z"}]"#;
    let (address, _) = serving(vec![("/api/sessions", body)]).await;
    let counts = reading(address).sweep().await;
    assert_eq!(counts.get("alive").map(|c| c.open), Some(3));
    assert!(!counts.contains_key("idle"), "a session with nothing to do");
}

#[tokio::test]
async fn a_second_read_inside_the_ttl_is_not_a_second_request() {
    // The whole reason a per-poll read is affordable: the front page polls every
    // five seconds, per client, and the service is on another machine.
    let body = r#"[{"id":"alive","open":1,"first_seen":"2026-08-08T10:00:00Z","last_seen":"2026-08-08T10:00:00Z"}]"#;
    let (address, asked) = serving(vec![("/api/sessions", body)]).await;
    let tasks = reading(address);
    assert_eq!(tasks.sweep().await.len(), 1);
    assert_eq!(tasks.sweep().await.len(), 1);
    assert_eq!(asked.load(Ordering::SeqCst), 1, "the cache was skipped");
}

#[tokio::test]
async fn a_service_that_will_not_answer_serves_the_last_known_counts() {
    // Stale beats blocking, and both beat failing. A console left running
    // through a reboot of isis shows the list it had, not an empty page.
    let body = r#"[{"id":"alive","open":2,"first_seen":"2026-08-08T10:00:00Z","last_seen":"2026-08-08T10:00:00Z"}]"#;
    let (address, _) = serving(vec![("/api/sessions", body)]).await;
    let tasks = reading(address);
    assert_eq!(tasks.sweep().await.get("alive").map(|c| c.open), Some(2));

    // Point it at a port with nothing behind it and expire the cache by hand is
    // not possible from here, so the failure is proven the other way: a reader
    // that has never had an answer returns nothing rather than raising.
    let dead = reading("127.0.0.1:1".parse().expect("addr"));
    assert!(dead.sweep().await.is_empty(), "no answer, and no panic");
}

#[tokio::test]
async fn a_list_carries_what_a_row_needs_and_not_the_prose() {
    let body = r#"[{"id":631,"repo":"memview","subject":"A slash command becomes prose","status":"open","assignee":{"kind":"nobody"},"detailed":true,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z"},
                   {"id":97,"repo":"memview","subject":"Finished already","status":"done","assignee":{"kind":"nobody"},"detailed":false,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z"}]"#;
    let (address, _) = serving(vec![("/api/tasks", body)]).await;
    let listed = reading(address).listed("whoever").await;
    assert_eq!(listed.len(), 2);
    // ⚠ The number arrives as a JSON number and is a string everywhere above
    // this — it is what a session calls a task in its own prose, `#631`.
    assert_eq!(listed[0].id, "631");
    assert_eq!(listed[0].subject, "A slash command becomes prose");
    assert_eq!(listed[0].status, "open");
    assert!(listed[0].detailed, "there is prose worth opening");
    // The service's own words, not a boolean of ours: `doing` is a third state
    // and the client sorts on it.
    assert_eq!(listed[1].status, "done");
    assert!(!listed[1].detailed);
}

#[tokio::test]
async fn a_task_with_no_prose_offers_none() {
    // Offering to open an empty sheet is worse than not offering.
    let empty = r#"{"id":97,"subject":"x","status":"open","assignee":{"kind":"nobody"},"detailed":false,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z","body":"   ","body_html":"","events":[]}"#;
    let (address, _) = serving(vec![("/api/tasks/97", empty)]).await;
    assert_eq!(reading(address).detail("97").await, None);
}

#[tokio::test]
async fn a_task_with_prose_returns_the_markdown_not_the_html() {
    // Both are sent. The console renders markdown itself — see `rendered.ts` —
    // and taking the HTML would put content outside that renderer's rules.
    // ⚠ `r###"…"###`, not one or two hashes. The body starts `"## Why`, and
    // that sequence closes BOTH `r#"…"#` and `r##"…"##`. Same trap as the
    // `## Context Usage` fixture in `console/tests/past.rs`.
    let full = r###"{"id":98,"subject":"x","status":"open","assignee":{"kind":"nobody"},"detailed":true,"created_at":"2026-08-08T10:00:00Z","updated_at":"2026-08-08T10:00:00Z","body":"## Why\n\nBecause.","body_html":"<h2>Why</h2>","events":[]}"###;
    let (address, _) = serving(vec![("/api/tasks/98", full)]).await;
    let said = reading(address).detail("98").await.expect("prose");
    assert!(said.starts_with("## Why"), "markdown, not html: {said}");
}

#[tokio::test]
async fn a_task_that_is_not_there_is_not_an_error() {
    let (address, _) = serving(vec![]).await;
    assert_eq!(reading(address).detail("404").await, None);
}
