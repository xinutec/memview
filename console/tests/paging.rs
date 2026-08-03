//! Reading a transcript backwards, one page at a time.
//!
//! The property under test is the one the previous design could not hold: pages
//! walked back from the newest are **disjoint and contiguous** — every event
//! appears exactly once, and together they are the whole file, in order.
//!
//! That was worth a test rather than a curl. The old interface took a count of
//! events the reader held and worked back from the end of the file; verified by
//! hand with a count typed at the command line, it looked right. The client sent
//! a count of *folded entries* instead — a different quantity of the same type —
//! and every page it got back was one it already had. Both numbers were `usize`,
//! so nothing anywhere could say so. A round trip is the only thing that could.

use std::path::{Path, PathBuf};

use console::past::page;
use console::protocol::{Event, Timed};

/// A transcript of `turns` exchanges, each one line of user and one of assistant
/// — the shape on disk, where a line is a message rather than a delta.
fn transcript(dir: &Path, turns: usize) -> PathBuf {
    let path = dir.join("paging.jsonl");
    let mut lines = Vec::new();
    for turn in 0..turns {
        lines.push(format!(
            r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"ask {turn}"}}]}}}}"#
        ));
        lines.push(format!(
            r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"answer {turn}"}}]}}}}"#
        ));
    }
    std::fs::write(&path, format!("{}\n", lines.join("\n"))).expect("transcript");
    path
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("console-paging-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("scratch");
    dir
}

/// Every page, newest first, by following the cursor to the start of the file.
fn all_pages(path: &Path) -> Vec<Vec<Timed>> {
    let mut pages = Vec::new();
    let mut cursor = None;
    loop {
        let page = page(path, cursor);
        let reached_start = page.from == 0;
        if !page.events.is_empty() {
            pages.push(page.events);
        }
        if reached_start {
            return pages;
        }
        assert!(
            pages.len() < 100,
            "the cursor is not advancing: stuck at {}",
            page.from
        );
        cursor = Some(page.from);
    }
}

fn text_of(events: &[Timed]) -> Vec<String> {
    events
        .iter()
        .filter_map(|timed| match &timed.event {
            Event::Text { text } | Event::Prompt { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn pages_walked_back_are_disjoint_and_contiguous() {
    // Enough turns to need several pages: REPLAY_EVENTS is 400, and each turn is
    // two events.
    let dir = scratch("walk");
    let path = transcript(&dir, 700);

    let pages = all_pages(&path);
    assert!(pages.len() > 1, "one page is not a test of paging");

    // Oldest first, they are the file — each event once, in order.
    let mut seen: Vec<String> = Vec::new();
    for events in pages.iter().rev() {
        seen.extend(text_of(events));
    }
    let expected: Vec<String> = (0..700)
        .flat_map(|turn| [format!("ask {turn}"), format!("answer {turn}")])
        .collect();
    assert_eq!(
        seen, expected,
        "every event exactly once, in the order the file has them"
    );
}

#[test]
fn the_newest_page_ends_at_the_newest_event() {
    // What a reader opens on. The last thing said is the thing they came for.
    let dir = scratch("newest");
    let path = transcript(&dir, 700);

    let newest = page(&path, None);
    assert_eq!(
        text_of(&newest.events).last().map(String::as_str),
        Some("answer 699")
    );
    assert!(newest.from > 0, "a long transcript has more behind it");
}

#[test]
fn a_short_transcript_is_one_page_that_says_so() {
    let dir = scratch("short");
    let path = transcript(&dir, 3);

    let only = page(&path, None);
    assert_eq!(only.from, 0, "nothing older, and the cursor says it");
    assert_eq!(text_of(&only.events).len(), 6);
}

#[test]
fn a_cursor_survives_the_file_growing() {
    // The reason a cursor replaced a count. A count taken from the end of the
    // file names a different place after the next turn; a byte offset does not.
    let dir = scratch("growing");
    let path = transcript(&dir, 500);

    let first = page(&path, None);
    let older = page(&path, Some(first.from));
    let before_growth = text_of(&older.events);

    // The session says something more, as it would while somebody is reading.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("append");
    use std::io::Write;
    writeln!(
        file,
        r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"later"}}]}}}}"#
    )
    .expect("write");

    let again = page(&path, Some(first.from));
    assert_eq!(
        text_of(&again.events),
        before_growth,
        "the same cursor names the same page after the file grew"
    );
}

#[test]
fn a_transcript_that_is_not_there_is_an_empty_page() {
    let dir = scratch("missing");
    let page = page(&dir.join("nothing.jsonl"), None);
    assert!(page.events.is_empty());
    assert_eq!(page.from, 0);
}
