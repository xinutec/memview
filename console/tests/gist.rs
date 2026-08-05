//! Reading a conversation down to what it is about, and reading a model's
//! answer back.
//!
//! The call itself is not exercised here — it costs a model and a process, and
//! what can go wrong with it is a spawn failing, which the caller already treats
//! as "no sentence this time". What these cover is the two ends: what is handed
//! to the model, and what is taken from what it says.

use std::collections::BTreeSet;
use std::path::Path;

use console::gist::{Gists, sentence};
use console::past::material;

/// A transcript in the shape the reader meets: opening plumbing that is not a
/// prompt, then a conversation, with tool calls between the sentences.
fn transcript(dir: &Path, id: &str, lines: &[String]) -> std::path::PathBuf {
    let folder = dir.join("project");
    std::fs::create_dir_all(&folder).expect("project dir");
    let path = folder.join(format!("{id}.jsonl"));
    std::fs::write(&path, lines.join("\n")).expect("transcript");
    path
}

fn said(who: &str, text: &str) -> String {
    format!(
        r#"{{"type":"{who}","message":{{"role":"{who}","content":[{{"type":"text","text":"{text}"}}]}}}}"#
    )
}

fn called(id: &str, command: &str) -> String {
    format!(
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"{id}","name":"Bash","input":{{"command":"{command}"}}}}]}}}}"#
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-gist-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

#[test]
fn both_ends_of_the_conversation_are_read() {
    // A conversation drifts, so neither end answers "what is this" alone: the
    // opening says what it was set up to do and the last exchanges say what it
    // has become.
    let root = scratch("both-ends");
    let mut lines = vec![
        r#"{"type":"system","cwd":"/home/example/Code/health"}"#.to_string(),
        said("user", "port the matcher gate to Lean"),
        said("assistant", "starting on the gate"),
    ];
    for n in 0..40 {
        lines.push(said("user", &format!("turn {n}")));
        lines.push(said("assistant", &format!("done {n}")));
    }
    let path = transcript(&root, "drifted", &lines);

    let found = material(&path, 6);
    assert_eq!(
        found.opening.as_deref(),
        Some("port the matcher gate to Lean"),
        "the opening is the first thing a person said, from the head of the file"
    );
    assert_eq!(found.recent.len(), 6, "capped at what was asked for");
    assert_eq!(
        found.recent.last().map(String::as_str),
        Some("agent: done 39"),
        "and the last of them is the newest"
    );
    assert!(
        found.recent.iter().all(|line| !line.contains("turn 0")),
        "the middle of a long conversation is not what it is about now"
    );
}

#[test]
fn the_plumbing_a_transcript_opens_with_is_not_the_instruction() {
    // ⚠ Measured on a real file: this console's own transcript begins with an
    // `/exit` caveat and two command echoes before anything anybody said. Taking
    // the first user line would summarise the plumbing.
    let root = scratch("plumbing");
    let path = transcript(
        &root,
        "opened",
        &[
            r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string(),
            said("user", "<command-name>/exit</command-name>"),
            said(
                "user",
                "<local-command-stdout>Compacted</local-command-stdout>",
            ),
            said("user", "do you know where our static analysis lives?"),
        ],
    );

    assert_eq!(
        material(&path, 4).opening.as_deref(),
        Some("do you know where our static analysis lives?")
    );
}

#[test]
fn the_tool_calls_are_left_out() {
    // They are most of the bytes in any working transcript and almost none of
    // the subject: a hundred `Bash` lines say a build was run, not what for.
    let root = scratch("no-tools");
    let path = transcript(
        &root,
        "busy",
        &[
            r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string(),
            said("user", "make the gate green"),
            called("toolu_1", "cargo test --workspace"),
            called("toolu_2", "cargo clippy"),
            said("assistant", "green"),
        ],
    );

    let found = material(&path, 20);
    assert_eq!(
        found.recent,
        vec!["them: make the gate green", "agent: green"]
    );
}

#[test]
fn a_transcript_that_says_nothing_yields_nothing() {
    // Rather than a prompt made of empty sections, which would have a model
    // inventing a subject from an instruction to summarise nothing.
    let root = scratch("empty");
    let path = transcript(
        &root,
        "blank",
        &[r#"{"type":"system","cwd":"/home/example/Code"}"#.to_string()],
    );

    let found = material(&path, 20);
    assert!(found.opening.is_none());
    assert!(found.recent.is_empty());
}

#[test]
fn the_answer_is_the_first_line_that_says_anything() {
    // A model asked for one sentence supplies a leading blank line often enough
    // to matter.
    assert_eq!(
        sentence("\n\nporting the matcher gate to Lean\n").as_deref(),
        Some("porting the matcher gate to Lean")
    );
}

#[test]
fn quotes_are_not_part_of_the_sentence() {
    // Returned despite being asked for none, and a quoted sentence on a card
    // reads as a citation of something somebody said.
    assert_eq!(
        sentence("\"porting the matcher gate\"").as_deref(),
        Some("porting the matcher gate")
    );
}

#[test]
fn nothing_said_is_no_sentence() {
    // Which the caller records as a failed attempt rather than as a summary, so
    // the conversation is tried again rather than left blank until it grows.
    assert!(sentence("").is_none());
    assert!(sentence("\n  \n").is_none());
}

/// A store holding one sentence per id, written the way the console writes it.
fn stored(name: &str, ids: &[&str]) -> (std::path::PathBuf, Gists) {
    let store = scratch(name).join("gists.json");
    let held: std::collections::BTreeMap<String, serde_json::Value> = ids
        .iter()
        .map(|id| {
            (
                (*id).to_string(),
                serde_json::json!({ "text": format!("about {id}"), "at": 1, "bytes": 10 }),
            )
        })
        .collect();
    std::fs::write(&store, serde_json::to_string(&held).expect("json")).expect("store");
    let gists = Gists::load(store.clone());
    (store, gists)
}

fn on_disk(store: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(store).expect("store");
    serde_json::from_str::<std::collections::BTreeMap<String, serde_json::Value>>(&text)
        .expect("json")
        .into_keys()
        .collect()
}

#[test]
fn a_conversation_that_is_gone_from_disk_loses_its_sentence() {
    // ⚠ The store only ever grew before this. Nothing on screen showed the
    // difference — a row comes from a walk of the disk and only then looks its
    // sentence up — so a deleted conversation left an entry that nothing could
    // ever read and nothing would ever remove.
    let (store, gists) = stored("forget-gone", &["kept", "deleted"]);

    gists.forget(&BTreeSet::from(["kept".to_string()]));

    assert_eq!(
        gists.all().into_keys().collect::<Vec<_>>(),
        vec!["kept".to_string()]
    );
    assert_eq!(
        on_disk(&store),
        vec!["kept".to_string()],
        "and written through, or a restart would read the dead entry back in"
    );
}

#[test]
fn an_empty_walk_is_not_a_reason_to_forget_everything() {
    // A directory that could not be read yields the same empty list as a
    // machine with no conversations on it, and from here the two look alike.
    // The cheap reading is the safe one: the true empty case has nothing to
    // forget, and the other would cost every sentence and a model call each to
    // write them again.
    let (store, gists) = stored("forget-empty", &["one", "two"]);

    gists.forget(&BTreeSet::new());

    assert_eq!(gists.all().len(), 2);
    assert_eq!(on_disk(&store).len(), 2);
}

#[test]
fn a_walk_that_matches_what_is_held_rewrites_nothing() {
    // The sweep runs on a timer and most of them have nothing to forget, so the
    // ordinary case must not rewrite the file — measured by the file's own
    // modification time, which is what a rewrite would move.
    let (store, gists) = stored("forget-same", &["one", "two"]);
    let before = std::fs::metadata(&store).expect("meta").modified().ok();

    gists.forget(&BTreeSet::from(["one".to_string(), "two".to_string()]));

    assert_eq!(gists.all().len(), 2);
    assert_eq!(
        std::fs::metadata(&store).expect("meta").modified().ok(),
        before,
        "untouched, rather than rewritten with identical contents"
    );
}
