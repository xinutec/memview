//! What a session is called to the other sessions on this machine.
//!
//! The name here is the one `ListAgents` prints and `SendMessage` resolves, and
//! it is not the title in the transcript: only a spawn writes it. The console
//! shows both so the gap between them is visible — see [`console::peers`].

use std::path::{Path, PathBuf};

use console::peers::named;

fn record(root: &Path, pid: u32, body: &str) {
    std::fs::create_dir_all(root).expect("root");
    std::fs::write(root.join(format!("{pid}.json")), body).expect("record");
}

fn scratch(what: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("console-peers-{}-{what}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

#[test]
fn a_session_is_known_by_the_name_in_its_own_record() {
    let root = scratch("named");
    record(
        &root,
        4242,
        r#"{"pid":4242,"name":"a-named-session","nameSource":"user"}"#,
    );
    assert_eq!(named(&root, 4242).as_deref(), Some("a-named-session"));
}

#[test]
fn a_name_the_cli_derived_is_still_the_name_peers_use() {
    // The whole point of showing this: `code-a7` is what a session is reachable
    // as, however little it says. Filtering by `nameSource` would hide exactly
    // the sessions worth noticing.
    let root = scratch("derived");
    record(
        &root,
        4243,
        r#"{"pid":4243,"name":"code-a7","nameSource":"derived"}"#,
    );
    assert_eq!(named(&root, 4243).as_deref(), Some("code-a7"));
}

#[test]
fn a_session_with_no_record_no_name_or_an_unreadable_one_is_simply_unknown() {
    let root = scratch("absent");
    assert_eq!(named(&root, 1), None, "no directory at all");
    record(&root, 2, r#"{"pid":2}"#);
    assert_eq!(named(&root, 2), None, "a record carrying no name");
    record(&root, 3, r#"{"pid":3,"name":"  "}"#);
    assert_eq!(named(&root, 3), None, "a name of nothing but spaces");
    record(&root, 4, "{not json");
    assert_eq!(named(&root, 4), None, "a record half-written");
}
