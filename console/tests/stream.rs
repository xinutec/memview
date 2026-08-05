//! What a client is sent when it says where it had got to.
//!
//! [`crate::session::Session::since`] is tested next door as arithmetic. What is
//! tested here is the wire: that a page saying `?after=` in the URL is actually
//! heard, and that a page saying nothing is told to start again. Those are two
//! different sentences on the socket — a `reset` event present or absent — and
//! the difference between them is a transcript kept and a transcript discarded.
//!
//! Over a real socket rather than by calling the handler, because the part that
//! has broken silently is not the logic; it is the plumbing that carries a number
//! from a URL into it. A test that hands the number to the handler directly would
//! have passed on every day this feature was broken.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use console::api;
use console::config::Config;
use console::protocol::Event;
use console::roster::Roster;
use console::session::Spawn;

use axum::http::HeaderMap;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn stub() -> String {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/stub-cli")
        .display()
        .to_string()
}

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
    }))
}

/// Serve the API on a port the OS picks, and say which.
async fn serve(roster: Arc<Roster>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, api::router(roster)).await;
    });
    addr
}

/// Ask for a session's stream and read what arrives in the first moment.
///
/// The read has to be bounded rather than read-to-end: an event stream does not
/// end, and the whole point of a resume that is up to date is that it says
/// *nothing* — so "the server went quiet" is the expected result and cannot be
/// waited for. The window closes once the socket has been idle, which is what
/// makes an empty answer distinguishable from a slow one.
async fn opening(addr: std::net::SocketAddr, path: &str) -> String {
    let mut socket = TcpStream::connect(addr).await.expect("connect");
    socket
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: console\r\n\r\n").as_bytes())
        .await
        .expect("request");
    let mut seen = Vec::new();
    let mut buffer = [0u8; 4096];
    // Long enough for a backlog to be written, short enough that the 15s
    // keep-alive can never land inside it and be mistaken for content.
    while let Ok(Ok(read)) =
        tokio::time::timeout(Duration::from_millis(400), socket.read(&mut buffer)).await
    {
        if read == 0 {
            break;
        }
        seen.extend_from_slice(&buffer[..read]);
    }
    String::from_utf8_lossy(&seen).into_owned()
}

/// The highest `id:` in what was read — where a client would say it had got to.
fn highest(body: &str) -> u64 {
    body.lines()
        .filter_map(|line| line.strip_prefix("id:"))
        .filter_map(|value| value.trim().parse::<u64>().ok())
        .max()
        .unwrap_or(0)
}

async fn until(session: &Arc<console::session::Session>, what: impl Fn(&[Event]) -> bool) {
    for _ in 0..100 {
        if what(&session.history()) {
            return;
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
async fn a_page_that_says_where_it_got_to_keeps_what_it_has() {
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("first").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    let addr = serve(roster).await;
    let events = format!("/api/sessions/{}/events", session.id);

    // Arriving with nothing: the transcript, and the instruction to empty the
    // page before taking it.
    let cold = opening(addr, &events).await;
    assert!(
        cold.contains("200 OK") && cold.contains("text/event-stream"),
        "the stream did not open at all: {cold}"
    );
    assert!(
        cold.contains("event: reset"),
        "a client holding nothing has to be told to start again: {cold}"
    );
    let held = highest(&cold);
    assert!(held > 0, "the first turn was numbered: {cold}");
    // And stamped. `Timed` flattens over the event, so this is one object on the
    // wire and a client reads `at` beside `kind` — a shape worth asserting,
    // because serde's flatten is the sort of thing that silently nests instead.
    assert!(
        cold.contains(r#""at":"#) && cold.contains(r#""kind":"#),
        "every event says when it happened: {cold}"
    );

    // Arriving saying where it got to, the way a page returning to a session it
    // had left does. Nothing has happened since, so the honest answer is silence
    // — and above all not `reset`, which would throw the transcript away.
    let warm = opening(addr, &format!("{events}?after={held}")).await;
    assert!(
        warm.contains("200 OK") && warm.contains("text/event-stream"),
        "the stream did not open at all: {warm}"
    );
    assert!(
        !warm.contains("event: reset"),
        "a client that named what it holds must keep it: {warm}"
    );
    assert_eq!(
        highest(&warm),
        0,
        "and be sent nothing it already had: {warm}"
    );

    // A turn happens while it is away, and exactly that turn is owed.
    session.send("second").await.expect("send");
    until(&session, |seen| {
        seen.iter()
            .filter(|e| matches!(e, Event::Turn { .. }))
            .count()
            == 2
    })
    .await;
    let missed = opening(addr, &format!("{events}?after={held}")).await;
    assert!(
        !missed.contains("event: reset"),
        "still resumable: {missed}"
    );
    assert!(highest(&missed) > held, "and sent what it missed: {missed}");
    assert!(
        !missed.contains("\"text\":\"first\""),
        "but not the turn it already had: {missed}"
    );
}

#[tokio::test]
async fn a_number_the_session_never_issued_starts_again() {
    // A console restarted under the same session id: the page holds numbers from
    // a run that no longer exists. Honouring them would hide every real event
    // until the count caught up — a session that had gone quiet, for minutes.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;
    let addr = serve(roster).await;
    let body = opening(
        addr,
        &format!("/api/sessions/{}/events?after=99999", session.id),
    )
    .await;
    assert!(
        body.contains("event: reset"),
        "a number from another run cannot be resumed: {body}"
    );
}

#[tokio::test]
async fn a_cursor_that_is_not_a_number_is_answered_rather_than_refused() {
    // Typed as a `u64`, the extractor would reject the request and the page would
    // show nothing at all. Replaying the transcript to a client we cannot
    // understand is the worse-looking answer and the better one.
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Started { .. }))
    })
    .await;
    let addr = serve(roster).await;
    let body = opening(
        addr,
        &format!("/api/sessions/{}/events?after=nonsense", session.id),
    )
    .await;
    assert!(body.contains("200 OK"), "it was refused: {body}");
    assert!(
        body.contains("event: reset"),
        "and treated as holding nothing: {body}"
    );
}

#[test]
fn the_browsers_own_number_wins_over_the_pages() {
    // Both are present on every reconnect of a stream opened with `?after=`: the
    // URL goes on naming where the page started, while the header names where it
    // got to. Preferring the URL would replay everything the connection had
    // already delivered.
    let mut headers = HeaderMap::new();
    headers.insert("last-event-id", "70".parse().expect("header"));
    assert_eq!(api::resume_from(&headers, Some("40")), Some(70));
    assert_eq!(api::resume_from(&HeaderMap::new(), Some("40")), Some(40));
    assert_eq!(api::resume_from(&HeaderMap::new(), None), None);
    // Unreadable at either end is absent, never a guess.
    assert_eq!(api::resume_from(&HeaderMap::new(), Some("")), None);
    let mut broken = HeaderMap::new();
    broken.insert("last-event-id", "not-a-number".parse().expect("header"));
    assert_eq!(api::resume_from(&broken, Some("40")), Some(40));
}
