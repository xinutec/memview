//! What a resumed session is called to the rest of the fleet.
//!
//! A name a console session carries is not decoration: `ListAgents` reads it and
//! `SendMessage` addresses it, so a session with no name of its own cannot be
//! reached by one. Every session this console starts runs in the same directory,
//! and the CLI derives a name from that directory when it is given none — so
//! eighteen of them came out as `code-` plus a hash, indistinguishable, and a
//! session told to reach `memview` reported truthfully that no such peer existed.
//!
//! The name exists all along; it just never travelled. `rename_session`, which is
//! what the console's rename endpoint sends, is the *title* — it reaches the
//! transcript and stops there. Only `-n` at spawn writes the registry the other
//! sessions read, which makes the resume the one moment this can be repaired.
//!
//! Its own test binary, because it sets `CLAUDE_PROJECTS_DIR`. That is
//! process-wide; see the note at the top of `cold.rs`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use console::config::Config;
use console::roster::Roster;
use console::session::Spawn;

/// The projects directory and the drop where the stub records its argument list,
/// both set once before any test spawns anything that reads the environment.
fn scratch() -> PathBuf {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let root = std::env::temp_dir().join(format!("console-naming-{}", std::process::id()));
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("project")).expect("scratch");
        std::fs::create_dir_all(root.join("argv")).expect("argv");
        // SAFETY: inside a `Once`, before any session exists to read it.
        unsafe {
            std::env::set_var("CLAUDE_PROJECTS_DIR", &root);
            std::env::set_var("CONSOLE_STUB_ARGV", root.join("argv"));
        }
    });
    root
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
            name: None,
            permission_mode: None,
        },
        static_dir: None,
        usage_url: None,
        gists: dir.join("gists.json"),
        modes: dir.join("modes.json"),
        drafts: dir.join("drafts.json"),
    }))
}

/// A transcript for `id`, named by `title` when there is one to give.
fn transcript(root: &std::path::Path, id: &str, title: Option<&str>) {
    let mut lines = vec![r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string()];
    if let Some(title) = title {
        lines.push(format!(
            r#"{{"type":"custom-title","customTitle":"{title}","sessionId":"{id}"}}"#
        ));
    }
    lines.push(
        r#"{"type":"user","timestamp":"2026-09-20T10:00:00Z","message":{"role":"user","content":[{"type":"text","text":"carry on"}]}}"#
            .to_string(),
    );
    std::fs::write(
        root.join("project").join(format!("{id}.jsonl")),
        format!("{}\n", lines.join("\n")),
    )
    .expect("transcript");
}

/// What the stub was called, once it has been called at all.
async fn called(root: &std::path::Path, id: &str) -> String {
    let drop = root.join("argv").join(format!("{id}.name"));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline {
        if let Ok(seen) = std::fs::read_to_string(&drop) {
            return seen;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the stub never recorded its arguments");
}

#[tokio::test]
async fn a_resumed_conversation_is_spawned_under_the_name_it_already_has() {
    let root = scratch();
    let dir = std::env::temp_dir();
    let id = "a-conversation-somebody-named";
    // Unique per run, because the name is READ OFF THE PROCESS TABLE.
    // `past::in_use` holds a conversation busy when any running `claude` carries
    // its id or its name as an argument — which is the guard working, not a fault.
    // This was first written as a fixed, readable name that happened to be one a
    // session on the developer's machine really answers to: the day that session
    // was up, the resume was refused and the test failed on the machine rather
    // than on anything in the tree.
    let name = format!("a-name-no-session-answers-to-{}", std::process::id());
    transcript(&root, id, Some(&name));

    let roster = roster(&dir);
    roster
        .resume(&dir.display().to_string(), id)
        .expect("resume");

    assert_eq!(
        called(&root, id).await,
        name,
        "the name in the transcript did not become the name its peers see"
    );
}

#[tokio::test]
async fn a_conversation_nobody_has_named_is_left_for_the_cli_to_name() {
    // The control, and it has to stay one: `-n ""` is not the same as no `-n` at
    // all. An empty name would take the session out of the CLI's own derive-and-
    // deduplicate path and leave it with nothing, which is worse than the hash.
    let root = scratch();
    let dir = std::env::temp_dir();
    let id = "a-conversation-nobody-named";
    transcript(&root, id, None);

    let roster = roster(&dir);
    roster
        .resume(&dir.display().to_string(), id)
        .expect("resume");

    assert_eq!(
        called(&root, id).await,
        "",
        "a nameless conversation was spawned under a name anyway"
    );
}
