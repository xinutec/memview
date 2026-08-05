//! Pictures sent from the phone, on the way to a session.
//!
//! The bytes come off a device and are handed to the API as whatever they claim
//! to be, so the tests here are mostly about disbelieving the claim.

use std::collections::BTreeSet;

use console::images::{LIMIT, keep, tidy};

/// The first bytes of each format, which is all the sniffer reads.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
const JPEG: &[u8] = b"\xff\xd8\xff\xe0\x00\x10JFIF";
const GIF: &[u8] = b"GIF89a\x01\x00";
const WEBP: &[u8] = b"RIFF\x24\x00\x00\x00WEBPVP8 ";
/// RIFF, and not an image: the container the WebP check exists to tell apart.
const AVI: &[u8] = b"RIFF\x24\x00\x00\x00AVI LIST";

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-images-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn a_picture_is_kept_where_it_can_be_opened_again() {
    // The copy is the whole reason the conversation is told a path: the image
    // itself is in the context only until a compaction drops it, and the session
    // asked about it an hour later needs somewhere to look.
    let root = scratch("kept");

    let held = keep(&root, "s1", "image/png", PNG, "2026-08-05-184700Z").expect("kept");

    assert_eq!(held.media_type, "image/png");
    assert_eq!(
        held.path,
        root.join("s1").join("2026-08-05-184700Z.png"),
        "named for the moment it arrived, under the session that was shown it"
    );
    assert_eq!(std::fs::read(&held.path).expect("read back"), PNG);
}

#[test]
fn what_it_is_comes_from_the_bytes_and_not_from_the_client() {
    // ⚠ **The media type is what the CLI is told and what the API believes.** A
    // mislabelled file would be sent as something it is not, and the refusal
    // would arrive minutes later, in another process, as a failed turn with no
    // reason attached.
    let root = scratch("sniffed");

    let held = keep(&root, "s1", "image/png", JPEG, "stamp").expect("kept");

    assert_eq!(held.media_type, "image/jpeg", "the bytes win");
    assert_eq!(
        held.path.extension().and_then(|it| it.to_str()),
        Some("jpg")
    );
}

#[test]
fn every_format_the_api_takes_is_recognised() {
    let root = scratch("formats");

    for (bytes, expected) in [
        (PNG, "image/png"),
        (JPEG, "image/jpeg"),
        (GIF, "image/gif"),
        (WEBP, "image/webp"),
    ] {
        let held = keep(&root, "s1", "", bytes, "stamp").expect("kept");
        assert_eq!(held.media_type, expected);
    }
}

#[test]
fn a_name_that_could_walk_out_of_the_directory_is_refused() {
    // ⚠ The session comes off the URL. The roster is asked for it first, so it is
    // an id this console holds — but this is the console's only write of a file
    // named from outside, and `Path::join` on `..` leaves the directory without
    // saying anything.
    let root = scratch("traversal");

    for name in ["../../etc", "a/b", ".."] {
        let refused = keep(&root, name, "image/png", PNG, "stamp").expect_err(name);
        assert!(refused.contains("not a session name"), "{refused}");
    }
    let refused = keep(&root, "s1", "image/png", PNG, "../escape").expect_err("stamp");
    assert!(refused.contains("not a filename"), "{refused}");
}

#[test]
fn a_riff_file_that_is_not_a_picture_is_refused() {
    // WebP is a RIFF container and so is an AVI. Reading the first four bytes
    // alone would have sent a video to an endpoint that takes images.
    let root = scratch("riff");

    let refused = keep(&root, "s1", "image/webp", AVI, "stamp").expect_err("not an image");

    assert!(refused.contains("not a PNG"), "{refused}");
}

#[test]
fn anything_that_is_not_an_image_is_refused_with_a_reason() {
    let root = scratch("refused");

    let refused = keep(&root, "s1", "image/png", b"just some words", "stamp").expect_err("no");

    assert!(refused.contains("not a PNG"), "{refused}");
    assert!(!root.join("s1").exists(), "and nothing was written");
}

#[test]
fn an_image_too_large_for_the_api_is_refused_here_rather_than_there() {
    // 5 MB is Anthropic's limit, not ours. The client scales before sending; this
    // is what answers a client that did not, in words rather than as a turn that
    // fails somewhere else a minute later.
    let root = scratch("large");
    let huge = [PNG, &vec![0u8; LIMIT]].concat();

    let refused = keep(&root, "s1", "image/png", &huge, "stamp").expect_err("too big");

    assert!(refused.contains("MB"), "{refused}");
}

#[test]
fn two_pictures_in_the_same_second_do_not_become_one() {
    // The stamp is the name, and a phone can send twice inside a second. The
    // older one is somebody's evidence, so it is not overwritten.
    let root = scratch("collision");

    let first = keep(&root, "s1", "image/png", PNG, "same").expect("first");
    let second = keep(&root, "s1", "image/jpeg", JPEG, "same").expect("second");

    assert_ne!(first.path, second.path);
    assert_eq!(std::fs::read(&first.path).expect("first still there"), PNG);
}

#[test]
fn the_message_puts_the_picture_before_the_question() {
    // ⚠ **Anthropic's own guidance, and not cosmetic**: a question read before
    // the thing it is about is answered from the question alone. The path rides
    // in the text so the session can open the picture again at full size after
    // the context has moved on.
    let line = console::protocol::prompt_with_image(
        "what is wrong with this?",
        "image/png",
        "AAAA",
        std::path::Path::new("/tmp/shot.png"),
    );

    let sent: serde_json::Value = serde_json::from_str(&line).expect("json");
    let content = sent["message"]["content"].as_array().expect("blocks");
    assert_eq!(content[0]["type"], "image");
    assert_eq!(content[0]["source"]["media_type"], "image/png");
    assert_eq!(content[0]["source"]["data"], "AAAA");
    assert_eq!(content[1]["type"], "text");
    let said = content[1]["text"].as_str().expect("text");
    assert!(said.starts_with("what is wrong with this?"), "{said}");
    assert!(said.contains("/tmp/shot.png"), "{said}");
}

#[test]
fn a_picture_sent_with_nothing_said_is_still_a_whole_message() {
    // The commonest message this endpoint carries: a screenshot and no words.
    // An empty text block would be a message that says the picture is not worth
    // looking at.
    let line = console::protocol::prompt_with_image(
        "   ",
        "image/jpeg",
        "AAAA",
        std::path::Path::new("/tmp/shot.jpg"),
    );

    let sent: serde_json::Value = serde_json::from_str(&line).expect("json");
    let said = sent["message"]["content"][1]["text"]
        .as_str()
        .expect("text");
    assert!(said.contains("showing you an image"), "{said}");
    assert!(said.contains("/tmp/shot.jpg"), "{said}");
}

/// A directory of pictures for each of `sessions`, as [`keep`] would leave them.
fn kept_for(root: &std::path::Path, sessions: &[&str]) {
    for session in sessions {
        keep(root, session, "image/png", PNG, "2026-08-05-184700Z").expect("kept");
    }
}

fn directories(root: &std::path::Path) -> Vec<String> {
    let mut found: Vec<String> = std::fs::read_dir(root)
        .expect("root")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();
    found
}

#[test]
fn pictures_go_when_the_conversation_they_belong_to_does() {
    // ⚠ Nothing removed one before this. Deleting a transcript left its pictures
    // behind for good — megabytes each, under a name that no longer answered to
    // anything, and no page that could ever show them again.
    let root = scratch("tidy-gone");
    kept_for(&root, &["alive", "deleted"]);

    let gone = tidy(&root, &BTreeSet::from(["alive".to_string()]));

    assert_eq!(gone, 1);
    assert_eq!(directories(&root), vec!["alive".to_string()]);
}

#[test]
fn nothing_to_keep_is_read_as_nothing_to_go_on() {
    // An unreadable projects directory yields the same empty set as a machine
    // with no conversations, and from here they look alike. One of those two
    // readings deletes every picture on the disk, so neither is acted on.
    let root = scratch("tidy-empty");
    kept_for(&root, &["one", "two"]);

    let gone = tidy(&root, &BTreeSet::new());

    assert_eq!(gone, 0);
    assert_eq!(directories(&root).len(), 2);
}

#[test]
fn a_directory_this_never_wrote_is_left_alone() {
    // The tidy deletes whole directories, so it only touches names it could have
    // created itself. Anything else under there arrived some other way and is
    // somebody else's to remove.
    let root = scratch("tidy-foreign");
    kept_for(&root, &["mine"]);
    std::fs::create_dir_all(root.join("not a session id")).expect("foreign");

    let gone = tidy(&root, &BTreeSet::from(["mine".to_string()]));

    assert_eq!(gone, 0);
    assert_eq!(
        directories(&root),
        vec!["mine".to_string(), "not a session id".to_string()]
    );
}
