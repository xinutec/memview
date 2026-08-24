//! Which side of the boundary each event lives on, and who is responsible for it.
//!
//! **The console has two sources of truth and only one of them is a file.** A
//! transcript records the conversation; the console's own state records what it
//! saw happen — a status, a question, a message written to a pipe. Since a cold
//! reader is seeded from the FILE (`api::cold`), every event kind that only the
//! console knows is one the seed must put back deliberately, or lose.
//!
//! ⚠ **That distinction lived in prose, and prose is why this file exists.**
//! 2026-08-24 the cold seed shipped with a doc comment listing the console-only
//! events as "`busy`, `accepted`, `started`". It missed `Ask`. A question is a
//! control request the CLI makes and no transcript holds one, so for ninety
//! minutes the session list said *waiting for you* while the conversation showed
//! nothing to answer, on a session that was genuinely blocked.
//!
//! The match below is exhaustive, so a new `Event` variant does not compile
//! until somebody says which side it is on — and, if it is the console's own,
//! either replays it or writes down what covers its absence.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use console::api;
use console::config::Config;
use console::protocol::{Event, read_recorded};
use console::roster::Roster;
use console::session::Spawn;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Where an event can come from.
enum Provenance {
    /// A transcript line produces it, so a seed read from the file carries it
    /// without anybody doing anything. Measured over 40 transcripts on
    /// 2026-08-24: `background command compacted context prompt text tool
    /// tool_result`, plus `shown`, which the sample held none of and
    /// `protocol::from_user` plainly produces for an image.
    Recorded,
    /// Only this console produces it. No file has it, so a cold seed either puts
    /// it back or it is gone.
    Runtime(Care),
}

enum Care {
    /// `api::cold` puts it back. Asserted below against a real socket.
    Replayed,
    /// Deliberately not replayed, because this says where a cold reader gets the
    /// same fact instead. A reason that names nothing is the bug this file is
    /// about, so write the source, not "not needed".
    Covered(&'static str),
}

/// ⚠ **Exhaustive on purpose.** A new variant fails to compile here, which is
/// the whole mechanism: the decision cannot be forgotten, only made.
fn provenance(event: &Event) -> Provenance {
    match event {
        // Written by the CLI into the transcript, so the page carries them.
        Event::Text { .. }
        | Event::Prompt { .. }
        | Event::Command { .. }
        | Event::Shown { .. }
        | Event::Context { .. }
        | Event::Tool { .. }
        | Event::ToolResult { .. }
        | Event::Background { .. }
        | Event::Compacted => Provenance::Recorded,

        // The console's own words.
        Event::Joined { .. } => Provenance::Runtime(Care::Replayed),
        Event::Ask { .. } => Provenance::Runtime(Care::Replayed),

        Event::Busy { .. } => Provenance::Runtime(Care::Covered(
            "Summary::busy, which the page reads until `caught-up` — see Held.spoken",
        )),
        Event::Turn { .. } => Provenance::Runtime(Care::Covered(
            "Summary::busy going quiet, and `caught-up` marking where the past stops",
        )),
        Event::Started { .. } => {
            Provenance::Runtime(Care::Covered("Summary::model and Summary::dir"))
        }
        Event::Limit { .. } => Provenance::Runtime(Care::Covered("Summary::limit and ::spent")),
        Event::Exited { .. } => Provenance::Runtime(Care::Covered("Summary::alive")),
        Event::Deaf { .. } => {
            Provenance::Runtime(Care::Covered("Summary::deaf, which the banner draws from"))
        }
        Event::Answered { .. } => Provenance::Runtime(Care::Covered(
            "the tool result the answer produced, which IS in the transcript",
        )),
        Event::Accepted { .. } => Provenance::Runtime(Care::Covered(
            "nothing — weighed with Pippijn 2026-08-24 and dropped: the marker \
             lives only between the write and the CLI's echo, and he had never \
             seen it. The one entry here that admits a loss rather than naming a \
             source; if the chip is ever missed, this is the line to change.",
        )),
        Event::Trouble { .. } => Provenance::Runtime(Care::Covered(
            "nothing — a past error message, of no use once the turn it broke is over",
        )),
    }
}

/// One transcript line per recorded kind, so the claim above is falsifiable
/// rather than asserted. Paths are `/home/example`: this repo is public.
fn recorded_fixtures() -> Vec<(&'static str, &'static str)> {
    vec![
        (
            "text",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hello"}]}}"#,
        ),
        (
            "prompt",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"do a thing"}]}}"#,
        ),
        (
            "tool",
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"toolu_1","name":"Bash","input":{"command":"ls /home/example"}}]}}"#,
        ),
        (
            "tool_result",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"toolu_1","content":"a.txt"}]}}"#,
        ),
        // ⚠ **Needs both blocks.** `protocol::from_user` only reads a picture
        // out of a message that carries an `image` AND the note the console
        // writes beside it; the image alone yields nothing, which this fixture
        // asserted wrongly at first and the test caught.
        (
            "shown",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGk="}},{"type":"text","text":"what is this (the image is also at /home/example/shot.png)"}]}}"#,
        ),
        (
            "compacted",
            r#"{"type":"system","subtype":"compact_boundary"}"#,
        ),
        // How full the window is, which rides on the assistant message's usage.
        (
            "context",
            r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-5","usage":{"input_tokens":10,"cache_read_input_tokens":90},"content":[{"type":"text","text":"hi"}]}}"#,
        ),
        // A slash command, which the CLI writes into the transcript wrapped in
        // its own tags rather than as an ordinary prompt.
        (
            "command",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<command-name>/compact</command-name><command-args></command-args>"}]}}"#,
        ),
        // A background task reporting its end, which arrives as a notification
        // in the user stream and names the call it belonged to.
        (
            "background",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"<task-notification><tool-use-id>toolu_bg</tool-use-id><status>completed</status></task-notification>"}]}}"#,
        ),
    ]
}

#[test]
fn every_kind_a_transcript_can_produce_is_declared_recorded() {
    for (kind, line) in recorded_fixtures() {
        let events = read_recorded(line);
        let found = events
            .iter()
            .find(|event| serde_json::to_value(event).unwrap()["kind"] == kind)
            .unwrap_or_else(|| {
                panic!("no {kind} came out of its own fixture line; got {events:?}")
            });
        assert!(
            matches!(provenance(found), Provenance::Recorded),
            "{kind} comes off a transcript line but is declared the console's own"
        );
    }
}

/// ⚠ **The reason has to name a source, and "nothing" has to say so out loud.**
/// A blank or hand-waving reason is how `Ask` went missing: the thought was had
/// and never written down.
#[test]
fn every_dropped_event_says_where_the_fact_comes_from_instead() {
    let dropped = [
        Event::Busy {
            status: "requesting".to_string(),
        },
        Event::Turn {
            cost_usd: 0.0,
            window: None,
            turns: 1,
            duration_ms: 0,
            stop: None,
        },
    ];
    for event in dropped {
        let Provenance::Runtime(Care::Covered(reason)) = provenance(&event) else {
            panic!("expected a covered runtime event");
        };
        assert!(
            reason.len() > 20,
            "the reason for dropping this is too short to be one: {reason:?}"
        );
    }
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

/// The behavioural half: a kind marked `Replayed` really does reach a reader who
/// arrives holding nothing. `joined` and `ask` are both covered by
/// `tests/cold.rs`; this asserts the declaration and the behaviour agree.
#[tokio::test]
async fn what_is_marked_replayed_reaches_a_cold_reader() {
    let scratch = std::env::temp_dir().join(format!("console-prov-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch");
    // SAFETY: one test in this binary reads it, and it is set before the session
    // that would consult it exists.
    unsafe { std::env::set_var("CLAUDE_PROJECTS_DIR", &scratch) };

    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    session.send("ask me").await.expect("send");
    for _ in 0..100 {
        if session
            .history()
            .iter()
            .any(|e| matches!(e, Event::Ask { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let folder = scratch.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    std::fs::write(
        folder.join(format!("{}.jsonl", session.id)),
        "{\"type\":\"assistant\",\"timestamp\":\"2026-08-24T10:00:00Z\",\"message\":{\"role\":\"assistant\",\"content\":[{\"type\":\"text\",\"text\":\"answer\"}]}}\n",
    )
    .expect("transcript");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, api::router(roster)).await;
    });

    let mut socket = TcpStream::connect(addr).await.expect("connect");
    socket
        .write_all(
            format!(
                "GET /api/sessions/{}/events HTTP/1.1\r\nHost: console\r\n\r\n",
                session.id
            )
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
    let cold = String::from_utf8_lossy(&seen);
    for kind in ["joined", "ask"] {
        assert!(
            cold.contains(&format!("\"kind\":\"{kind}\"")),
            "{kind} is declared Replayed and did not reach a cold reader"
        );
    }
}
