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
//! is the console's own recent words: `started`, `busy` and `accepted` are in no
//! transcript, and a reader arriving cold no longer gets them. Both are asserted,
//! because a change that quietly stopped doing the second would look exactly like
//! this one passing.
//!
//! ⚠ **`Ask` is the one console-only event that IS put back**, and it has its own
//! test here. A display that goes missing is a display; a question that goes
//! missing stops the session. `tests/provenance.rs` is where that distinction is
//! made once for every event kind, rather than remembered.

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

/// A pipe whose far end this test keeps open — an adopted session reads end of
/// file at once otherwise and calls itself over.
fn carried_pipe() -> (std::os::fd::RawFd, std::os::fd::RawFd) {
    let mut ends = [0 as libc::c_int; 2];
    // SAFETY: pipe(2) writes two descriptors into an array this test owns.
    assert_eq!(unsafe { libc::pipe(ends.as_mut_ptr()) }, 0, "pipe");
    (ends[0], ends[1])
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
async fn compressed(addr: std::net::SocketAddr, path: &str) -> (TcpStream, Vec<u8>) {
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
    // ⚠ **Raw, not a lossy string.** These bytes are a gzip member, and
    // `from_utf8_lossy` replaces every invalid sequence with U+FFFD — which
    // decodes to nothing at all. Read back as text for the header check only.
    (socket, seen)
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
    let reply = String::from_utf8_lossy(&head).to_lowercase();
    assert!(
        reply.contains("content-encoding: gzip"),
        "the stream was not compressed at all, so this proves nothing: {}",
        &reply[..reply.len().min(400)]
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

    // The whole gzip member so far: its header arrived with the seed, so the
    // decoder is fed from the start of the body rather than from this read alone.
    let mut whole = head;
    whole.extend_from_slice(&buffer[..arrived]);
    let body = whole
        .windows(4)
        .position(|four| four == b"\r\n\r\n")
        .map(|at| whole.split_off(at + 4))
        .expect("no end of headers in the reply");
    let body = dechunked(&body);
    let mut plain = Vec::new();
    // Cut mid-member, which is the normal case here — the stream has not ended
    // and will not — so a decode error AFTER real output is the expected ending.
    let _ = std::io::copy(&mut flate2::read::GzDecoder::new(&body[..]), &mut plain);
    let text = String::from_utf8_lossy(&plain);
    assert!(
        text.contains("second"),
        "the compressor flushed something, but not the event: {:?}",
        &text[text.len().saturating_sub(200)..]
    );
}

#[tokio::test]
async fn a_question_still_standing_is_put_to_a_reader_who_arrives_cold() {
    let scratch = projects();
    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("ask me").await.expect("send");
    until(&session, |seen| {
        seen.iter().any(|e| matches!(e, Event::Ask { .. }))
    })
    .await;
    // A transcript with no question in it, which is every transcript: an `Ask`
    // is a control request the CLI made and the file records none.
    transcript(&scratch, &session.id, 600);

    let addr = serve(roster).await;
    let cold = opening(addr, &format!("/api/sessions/{}/events", session.id)).await;

    // ⚠ **The list and the conversation have to agree.** `Summary::asked` is
    // computed from the session's own state and says *waiting for you* whatever
    // the stream carried, so a seed that dropped the question left the front
    // page asking and the session showing nothing to answer — seen on
    // `hardware`, 2026-08-24.
    assert!(
        cold.contains("\"kind\":\"ask\""),
        "the pending question was not put to a cold reader: {}",
        &cold[cold.len().saturating_sub(600)..]
    );
    assert!(
        cold.contains("which way"),
        "the question arrived without what it asked"
    );
}

#[tokio::test]
async fn a_sessions_name_comes_from_the_head_of_its_transcript_not_the_last_page() {
    // ⚠ **The ordering is the whole test.** `asked` binds to the first `Prompt`
    // seen while nothing is bound, and `adopt` seeds from the transcript right
    // after building its state — so a label restored AFTER the seed, or not at
    // all, is taken by whatever prompt happens to start the last page. Measured
    // on the phone 2026-08-24: `hardware`'s subtitle changed across two upgrades
    // an hour apart with nobody touching it (memview #1146).
    let scratch = projects();
    let id = "a-session-with-a-page-full-of-prompts";
    let folder = scratch.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    // ⚠ **Opens with plumbing, as real transcripts do.** A `<local-command-caveat>`
    // is not something anybody asked, and `read_recorded` already declines to
    // make it a prompt — so a reader that took the first LINE would name the
    // session after a caveat block.
    let mut lines = vec![
        r#"{"type":"user","timestamp":"2026-08-24T10:59:00Z","message":{"role":"user","content":[{"type":"text","text":"<local-command-caveat>ignore me</local-command-caveat>"}]}}"#.to_string(),
        r#"{"type":"user","timestamp":"2026-08-24T10:59:30Z","message":{"role":"user","content":[{"type":"text","text":"look at the fleet's disks"}]}}"#.to_string(),
    ];
    lines.extend((0..20).map(|n| {
        format!(
            r#"{{"type":"user","timestamp":"2026-08-24T11:00:{n:02}Z","message":{{"role":"user","content":[{{"type":"text","text":"a later prompt {n}"}}]}}}}"#
        )
    }));
    std::fs::write(
        folder.join(format!("{id}.jsonl")),
        format!("{}\n", lines.join("\n")),
    )
    .expect("transcript");

    let (_stdin_read, stdin) = carried_pipe();
    let (stdout, _stdout_write) = carried_pipe();
    let (stderr, _stderr_write) = carried_pipe();
    let session = console::session::Session::adopt(
        id.to_string(),
        std::env::temp_dir(),
        std::process::id(),
        console::session::Fds {
            stdin,
            stdout,
            stderr,
        },
        console::session::Tally {
            started: 1_754_000_000,
            model: None,
            mode: None,
            // Deliberately WRONG, as an earlier image's value would be: the
            // head of the file has to win, or a subtitle once corrupted stays
            // corrupted for ever.
            asked: Some("a later prompt 7".into()),
            cost_usd: 0.0,
            window: None,
            limit: None,
            busy: None,
            pending: Default::default(),
            background: [(
                "toolu_long_runner".to_string(),
                console::protocol::Called {
                    tool: "Monitor".into(),
                    label: Some("errors in deploy.log".into()),
                    task: Some("bq1abc".into()),
                },
            )]
            .into_iter()
            .collect(),
            spent: Default::default(),
            counted: Default::default(),
        },
    )
    .expect("adopt");

    // ⚠ **Started before the page, and still running.** `execve` leaves the
    // children alone, so a monitor armed an hour ago is still armed — but its
    // `tool` event is long out of the replayed page, so a re-seed forgets it and
    // the card says nothing is in flight. Carried, like the question and the
    // name, because it is a fact about the present.
    assert_eq!(
        session.summary().background,
        1,
        "a background task older than one page was lost across the handover"
    );

    assert_eq!(
        session.summary().asked.as_deref(),
        Some("look at the fleet's disks"),
        "the name came from the last page or a stale tally, not the head of the file"
    );
}

#[tokio::test]
async fn a_conversation_continued_from_a_compacted_one_claims_no_origin() {
    // ⚠ **`None` is the answer, not a fallback.** When a conversation runs out
    // of context the CLI opens a fresh transcript with a summary and a
    // `This session is being continued…` message — the harness talking, not
    // Pippijn. Measured on `heatcam` 2026-08-24: its first user text is 447 kB
    // in and is exactly that. Leaving a recent prompt in its place is the false
    // claim this whole change repairs, so the honest answer is to say nothing
    // and let the session's own name identify it.
    let scratch = projects();
    let id = "a-conversation-that-was-compacted";
    let folder = scratch.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    std::fs::write(
        folder.join(format!("{id}.jsonl")),
        format!(
            "{}\n{}\n",
            r#"{"type":"user","timestamp":"2026-08-24T10:00:00Z","message":{"role":"user","content":[{"type":"text","text":"This session is being continued from a previous conversation that ran out of context. The summary below covers..."}]}}"#,
            r#"{"type":"user","timestamp":"2026-08-24T10:05:00Z","message":{"role":"user","content":[{"type":"text","text":"a much later prompt"}]}}"#,
        ),
    )
    .expect("transcript");

    let (_stdin_read, stdin) = carried_pipe();
    let (stdout, _stdout_write) = carried_pipe();
    let (stderr, _stderr_write) = carried_pipe();
    let session = console::session::Session::adopt(
        id.to_string(),
        std::env::temp_dir(),
        std::process::id(),
        console::session::Fds {
            stdin,
            stdout,
            stderr,
        },
        console::session::Tally {
            started: 1_754_000_000,
            model: None,
            mode: None,
            // What an earlier image had wrongly taken for the name.
            asked: Some("a much later prompt".into()),
            cost_usd: 0.0,
            window: None,
            limit: None,
            busy: None,
            pending: Default::default(),
            background: Default::default(),
            spent: Default::default(),
            counted: Default::default(),
        },
    )
    .expect("adopt");

    assert_eq!(
        session.summary().asked,
        None,
        "a continued conversation claimed an origin it does not have"
    );

    // ⚠ **And it has to STAY none.** Three places name a session after a prompt
    // when it has no name — right for a session with no transcript, wrong here:
    // the file was read and had no beginning to give. Without `origin_read` the
    // blank lasts until the next message and the false claim comes straight
    // back, an hour later and invisibly.
    session.send("something said later").await.expect("send");
    assert_eq!(
        session.summary().asked,
        None,
        "the next thing said was taken for the beginning of the conversation"
    );
}

/// The payload out of an HTTP chunked body, as far as it goes.
///
/// ⚠ **Needed before any decoding.** The stream is `Transfer-Encoding: chunked`,
/// so the bytes on the socket are `size CRLF data CRLF` — feeding those to a gzip
/// decoder hands it a hex length where a header should be and yields nothing at
/// all, which reads exactly like a compressor that never flushed. Stops at the
/// first incomplete chunk, because a live stream is always cut somewhere.
fn dechunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    loop {
        let Some(eol) = rest.windows(2).position(|two| two == b"\r\n") else {
            return out;
        };
        let Ok(head) = std::str::from_utf8(&rest[..eol]) else {
            return out;
        };
        // A chunk extension after `;` is legal and unused here.
        let size = head.split(';').next().unwrap_or("").trim();
        let Ok(size) = usize::from_str_radix(size, 16) else {
            return out;
        };
        if size == 0 {
            return out;
        }
        let from = eol + 2;
        let Some(chunk) = rest.get(from..from + size) else {
            return out;
        };
        out.extend_from_slice(chunk);
        rest = &rest[(from + size + 2).min(rest.len())..];
    }
}
