//! What a session learns while its process says nothing.
//!
//! The transcript is not only the record of a conversation, it is the only place
//! some of it is ever written. A compaction is the case that matters: the CLI
//! files a boundary and announces it on no stream, so a console watching stdout
//! sees a compaction as silence.
//!
//! `past::counted` has always reported the boundary correctly, and there is a
//! test next door that says so. What was never tested is whether anybody CALLS
//! it while a session is idle — the read was tied to the end of a turn, so a
//! conversation compacted and then left waiting never read its own boundary.
//! Found live: `home` showed 258,318 tokens for ninety minutes, which was the
//! fullness of a conversation that had been replaced, beside an exchange count
//! the boundary had reset to zero. Both drawn exactly as a live figure is.
//!
//! ⚠ **Its own test binary, because it sets `CLAUDE_PROJECTS_DIR`.** That is
//! process-wide, and a session reads it whenever it recounts, so a sibling test
//! sharing the process would have its transcripts looked for somewhere else.

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

/// The transcript the CLI would have written for the turn just taken.
///
/// Written by the test because the stand-in CLI does not keep one — and the file
/// is the whole point here: it is where a compaction is announced.
fn transcript(root: &std::path::Path, id: &str) -> PathBuf {
    let folder = root.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let path = folder.join(format!("{id}.jsonl"));
    std::fs::write(
        &path,
        "{\"type\":\"system\",\"cwd\":\"/home/example/Code\"}\n",
    )
    .expect("transcript");
    path
}

#[tokio::test]
async fn a_compaction_is_noticed_while_the_process_says_nothing() {
    let scratch = std::env::temp_dir().join("console-idle-recount");
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).expect("scratch");
    // SAFETY: this test binary holds one test, so nothing else in the process is
    // reading the environment while it is set. See the note at the top.
    unsafe { std::env::set_var("CLAUDE_PROJECTS_DIR", &scratch) };

    let dir = std::env::temp_dir();
    let roster = roster(&dir);
    let session = roster.start(&dir.display().to_string()).expect("start");
    for _ in 0..100 {
        if session
            .history()
            .iter()
            .any(|e| matches!(e, Event::Started { .. }))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let id = session.summary().id;
    let path = transcript(&scratch, &id);

    // A turn that leaves the context half a million tokens full, reported the
    // way the real CLI reports it: on the assistant message, over the stream.
    session.send("say how full").await.expect("send");
    let full = wait_for(&session, |context| context.is_some()).await;
    assert_eq!(full, Some(547_869), "the fullness came off the stream");

    // Now compact it, and say nothing at all on the stream — no turn, no text,
    // no exit. This is the whole test: the only thing that changes is the file.
    let mut text = std::fs::read_to_string(&path).expect("read");
    text.push_str("{\"type\":\"system\",\"subtype\":\"compact_boundary\"}\n");
    std::fs::write(&path, text).expect("append");

    let after = wait_for(&session, |context| context.is_none()).await;
    assert_eq!(
        after, None,
        "the pre-compaction figure was still being shown; \
         nothing re-read the transcript while the session was idle"
    );
}

/// Poll the summary until the fullness is what the caller is waiting for.
///
/// Polled rather than slept on, so a slow machine makes this slower instead of
/// flaky — and bounded well above `RECOUNT_EVERY`, because what is being tested
/// is that the read happens at all, not how promptly.
async fn wait_for(
    session: &Arc<console::session::Session>,
    what: impl Fn(Option<u64>) -> bool,
) -> Option<u64> {
    for _ in 0..300 {
        let context = session.summary().context;
        if what(context) {
            return context;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "waited 30s; fullness stayed at {:?}",
        session.summary().context
    );
}
