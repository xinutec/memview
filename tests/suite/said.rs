//! What the author said a command was for, read off a transcript line.
//!
//! The first instrument of `docs/concept-model.md`: a parallel corpus of
//! (command, stated intent) pairs, mined so a lifted concept has something that
//! is not this code to be checked against.
//!
//! ⚠ **The boundary these tests exist to hold is that a description is a CLAIM,
//! not evidence.** It is prose by the same author at the same moment, and the
//! reader must never consult it to decide what a command did. Nothing here
//! asserts a description is true; they assert it is carried faithfully and kept
//! where a static reader will not trip over it.

use memview::agents::{BashCall, bash_calls_with_ids};

/// One transcript line carrying one `Bash` call, spelled the way the CLI writes
/// it — the call inside `message.content`, the cwd and stamp on the row.
fn line(command: &str, description: Option<&str>) -> String {
    let input = match description {
        Some(said) => serde_json::json!({ "command": command, "description": said }),
        None => serde_json::json!({ "command": command }),
    };
    serde_json::json!({
        "timestamp": "2026-09-03T08:00:00.000Z",
        "cwd": "/home/example/Code",
        "message": { "content": [
            { "type": "tool_use", "name": "Bash", "id": "toolu_01", "input": input }
        ]}
    })
    .to_string()
}

fn only(text: &str) -> BashCall {
    let read = bash_calls_with_ids(text.as_bytes()).expect("a Bash line");
    read.calls.into_iter().next().expect("one call")
}

#[test]
fn the_stated_intent_is_carried_beside_the_command() {
    let got = only(&line("cargo test -p reader", Some("Run the reader tests")));
    assert_eq!(got.command, "cargo test -p reader");
    assert_eq!(got.description.as_deref(), Some("Run the reader tests"));
}

/// ⚠ **Absence is a fact, not a blank.** The report's first figure is the share
/// of calls that said anything at all — 187,701 of 197,126, 95.2%, mined
/// 2026-09-03 — and it needs the missing ones to stay distinguishable from empty
/// ones rather than defaulted to a string. Read that share off a run: a
/// six-transcript sample of the same corpus said 97.6%.
#[test]
fn a_call_that_said_nothing_carries_no_description() {
    assert_eq!(only(&line("ls", None)).description, None);
}

/// An empty string is something the author wrote, and `None` is something they
/// did not. Collapsing the two would put an unwritten description into the
/// numerator of the presence figure.
#[test]
fn an_empty_description_is_not_the_same_as_an_absent_one() {
    assert_eq!(only(&line("ls", Some(""))).description.as_deref(), Some(""));
}

/// ⚠ **The command survives a description this cannot read.** A malformed or
/// non-string `description` must cost the intent and nothing else — dropping the
/// call would take a command out of the corpus over prose that has no bearing on
/// what ran, which is the boundary this whole file is about.
#[test]
fn a_description_that_is_not_a_string_costs_only_itself() {
    let text = serde_json::json!({
        "timestamp": "2026-09-03T08:00:00.000Z",
        "cwd": "/home/example/Code",
        "message": { "content": [
            { "type": "tool_use", "name": "Bash", "id": "toolu_01",
              "input": { "command": "ls", "description": { "not": "a string" } } }
        ]}
    })
    .to_string();
    let got = only(&text);
    assert_eq!(got.command, "ls");
    assert_eq!(got.description, None);
}

/// Every call on a line keeps its own, which is what makes `(at, cmd)` a key
/// rather than a line pointer: one transcript line can carry several calls, and
/// they share a timestamp.
#[test]
fn each_call_on_a_line_keeps_its_own_intent() {
    let text = serde_json::json!({
        "timestamp": "2026-09-03T08:00:00.000Z",
        "cwd": "/home/example/Code",
        "message": { "content": [
            { "type": "tool_use", "name": "Bash", "id": "toolu_01",
              "input": { "command": "git status", "description": "Check the tree" } },
            { "type": "tool_use", "name": "Bash", "id": "toolu_02",
              "input": { "command": "git log -1", "description": "Read the last commit" } }
        ]}
    })
    .to_string();
    let read = bash_calls_with_ids(text.as_bytes()).expect("a Bash line");
    let said: Vec<_> = read
        .calls
        .iter()
        .map(|call| call.description.as_deref())
        .collect();
    assert_eq!(said, [Some("Check the tree"), Some("Read the last commit")]);
}
