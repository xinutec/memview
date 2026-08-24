//! What a reader who holds nothing is sent.
//!
//! ⚠ **Its own test binary, because it sets `CLAUDE_PROJECTS_DIR`.** That is
//! process-wide and a session reads it whenever it recounts — the same reason
//! `idle.rs` stands alone. Both tests here share ONE directory, set once: two
//! tests setting it to two scratch paths is a race that reads as this feature
//! not working, and did.
//!
//! The claim under test is a trade, so both halves are pinned here. A cold
//! stream carries the transcript's last page and stops there, rather than the
//! whole scrollback the console has been keeping for its own resumes — that is
//! the megabyte a phone on a bad connection was waiting through. What it costs
//! is the console's own recent words: `started`, `busy`, `sent` are in no
//! transcript, and a reader arriving cold no longer gets them. Both are asserted,
//! because a change that quietly stopped doing the second would look exactly like
//! this one passing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use console::api;
use console::config::Config;
use console::protocol::Event;
use console::roster::Roster;
use console::session::Spawn;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// The one projects directory this binary uses, set before any test runs.
///
/// Shared rather than per-test: the variable is process-wide, so two tests each
/// naming their own would take turns pointing the other's session at a directory
/// with no transcript in it — which looks exactly like the seed falling back to
/// the log.
fn projects() -> PathBuf {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let scratch = std::env::temp_dir().join(format!("console-cold-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch");
        // SAFETY: inside a `Once`, before either test has spawned anything that
        // reads the environment. See the note at the top.
        unsafe { std::env::set_var("CLAUDE_PROJECTS_DIR", &scratch) };
    });
    scratch
}

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
        usage_url: None,
        gists: dir.join("gists.json"),
        modes: dir.join("modes.json"),
    }))
}

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

/// The stream's first moment. Bounded rather than read to the end: an event
/// stream does not finish, so quiet is an answer and has to be waited out.
async fn opening(addr: std::net::SocketAddr, path: &str) -> String {
    let mut socket = TcpStream::connect(addr).await.expect("connect");
    socket
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: console\r\n\r\n").as_bytes())
        .await
        .expect("request");
    let mut seen = Vec::new();
    let mut buffer = [0u8; 8192];
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

/// A transcript of `turns` exchanges, each numbered so a page can be recognised
/// by which end of the file it came from.
fn transcript(root: &std::path::Path, id: &str, turns: usize) -> PathBuf {
    let folder = root.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let path = folder.join(format!("{id}.jsonl"));
    let mut lines = vec!["{\"type\":\"system\",\"cwd\":\"/home/example/Code\"}".to_string()];
    for turn in 0..turns {
        // ⚠ **Stamped, and the test is empty without it.** With no `timestamp`
        // the marker and the last line both carry `None`, and an assertion that
        // they match passes by both being absent — a check that cannot fail.
        // Times are a fixed minute apart from a fixed start, so nothing here
        // reads the clock.
        let minute = 30 + turn / 60;
        let second = turn % 60;
        lines.push(format!(
            r#"{{"type":"assistant","timestamp":"2026-08-24T10:{minute:02}:{second:02}Z","message":{{"role":"assistant","content":[{{"type":"text","text":"answer {turn}"}}]}}}}"#
        ));
    }
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("transcript");
    path
}

async fn until(session: &Arc<console::session::Session>, what: impl Fn(&[Event]) -> bool) {
    for _ in 0..100 {
        if what(&session.history()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("waited 5s and it never happened");
}

/// How many events the stream carried — `data:` lines, which is one per event.
fn events(body: &str) -> usize {
    body.lines()
        .filter(|line| line.starts_with("data:"))
        .count()
}

fn highest(body: &str) -> u64 {
    body.lines()
        .filter_map(|line| line.strip_prefix("id:"))
        .filter_map(|value| value.trim().parse::<u64>().ok())
        .max()
        .unwrap_or(0)
}

#[tokio::test]
async fn a_reader_holding_nothing_is_sent_the_end_of_the_transcript_and_no_more() {
    let scratch = projects();
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("first").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    // Written after the session is up, so the file is plainly not what seeded
    // this log: the console has never read it.
    transcript(&scratch, &session.id, 600);

    let addr = serve(roster).await;
    let cold = opening(addr, &format!("/api/sessions/{}/events", session.id)).await;

    assert!(
        cold.contains("event: reset"),
        "a cold reader is told to start again: {cold}"
    );
    // The newest end of the file, and not the oldest: a page, not the whole
    // thing. This is the byte count the phone was waiting through.
    assert!(
        cold.contains("answer 599"),
        "the newest exchange did not arrive"
    );
    assert!(
        !cold.contains("\"answer 0\""),
        "the whole file arrived — the page is not bounded"
    );
    // One page of the six hundred turns on disk, plus the `joined` that ends it
    // — `past::REPLAY_EVENTS` is 400. Not an exact count: whatever the session
    // says while the stream is open rides along behind the marker, and a bound is
    // what is being claimed anyway.
    assert!(
        (400..500).contains(&events(&cold)),
        "expected a page and its marker out of 600 turns, got {} events",
        events(&cold)
    );
    // ⚠ **Dated where the page ends, not when this reader arrived.** Stamped
    // with the clock it sat at the foot of the transcript as the newest thing
    // in it, and moved every time the session was opened — a fact about the
    // connection wearing the clothes of a fact about the conversation.
    let marker = cold
        .lines()
        .find(|line| line.contains("\"kind\":\"joined\""))
        .expect("no marker between the file and now");
    let last_page_line = cold
        .lines()
        .rfind(|line| line.starts_with("data:") && line.contains("\"kind\":\"text\""))
        .expect("the page carried no text at all");
    let at_of = |line: &str| {
        line.split("\"at\":")
            .nth(1)
            .and_then(|rest| rest.split(&[',', '}'][..]).next())
            .map(str::to_string)
    };
    assert_eq!(
        at_of(marker),
        at_of(last_page_line),
        "the marker is dated on its own clock rather than on the page it ends"
    );

    // The cursor for what came before, which is the only thing that knows the
    // conversation is longer than the page.
    assert!(
        cold.contains("\"kind\":\"joined\""),
        "no marker between the file and now"
    );
    assert!(
        !cold.contains("\"from\":0"),
        "the marker claims the page reached the start of a 600-turn file"
    );
    // The trade, stated: the console's own words about itself are in no
    // transcript and are not replayed to a cold reader. `started` is the one
    // this log certainly holds.
    assert!(
        session
            .history()
            .iter()
            .any(|e| matches!(e, Event::Started { .. })),
        "the log should hold the session's own start"
    );
    assert!(
        !cold.contains("\"kind\":\"started\""),
        "a console-only event was replayed from the log after all"
    );
}

#[tokio::test]
async fn the_number_the_seed_ends_on_resumes_without_sending_the_page_again() {
    let scratch = projects();
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("first").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    transcript(&scratch, &session.id, 600);

    let addr = serve(roster).await;
    let events_url = format!("/api/sessions/{}/events", session.id);
    let cold = opening(addr, &events_url).await;
    let held = highest(&cold);
    assert!(held > 0, "the seed ended on no number at all: {cold}");

    let again = opening(addr, &format!("{events_url}?after={held}")).await;
    assert!(
        !again.contains("event: reset"),
        "the number the seed ended on was not honoured: {again}"
    );
    assert!(
        !again.contains("answer 599"),
        "the page was sent a second time to a reader that already held it"
    );
}

/// Open a stream asking for compression, and hand back the socket still open.
///
/// The reply is read only as far as the seed, because what is being measured is
/// what happens *after* it: whether a live event has to wait for a compressor's
/// buffer to fill before any of it reaches the wire.
async fn compressed(addr: std::net::SocketAddr, path: &str) -> (TcpStream, String) {
    let mut socket = TcpStream::connect(addr).await.expect("connect");
    socket
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: console\r\nAccept-Encoding: gzip\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("request");
    let mut seen = Vec::new();
    let mut buffer = [0u8; 8192];
    while let Ok(Ok(read)) =
        tokio::time::timeout(Duration::from_millis(400), socket.read(&mut buffer)).await
    {
        if read == 0 {
            break;
        }
        seen.extend_from_slice(&buffer[..read]);
    }
    (socket, String::from_utf8_lossy(&seen).into_owned())
}

#[tokio::test]
async fn a_live_event_is_not_held_back_by_the_compressor() {
    let scratch = projects();
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("first").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Turn { .. }))
    })
    .await;
    transcript(&scratch, &session.id, 600);

    let addr = serve(roster).await;
    let (mut socket, head) =
        compressed(addr, &format!("/api/sessions/{}/events", session.id)).await;
    assert!(
        head.to_lowercase().contains("content-encoding: gzip"),
        "the stream was not compressed at all, so this proves nothing: {}",
        &head[..head.len().min(400)]
    );

    // Now make the session say one thing, and see whether any of it reaches the
    // socket. Not decoded: the claim is about flushing, and bytes arriving at all
    // is the whole of it. A compressor holding its buffer would send nothing here
    // until the next event — which on a real session can be minutes.
    session.send("second").await.expect("send");
    let mut buffer = [0u8; 8192];
    let arrived = tokio::time::timeout(Duration::from_secs(3), socket.read(&mut buffer))
        .await
        .expect("nothing reached the socket in 3s — the compressor is buffering the stream")
        .expect("read");
    assert!(
        arrived > 0,
        "the stream closed instead of carrying the event"
    );
}
