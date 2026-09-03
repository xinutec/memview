//! Pictures sent from the phone, on the way to a session.
//!
//! The bytes come off a device and are handed to the API as whatever they claim
//! to be, so the tests here are mostly about disbelieving the claim.

use std::collections::BTreeSet;

use console::images::{LIMIT, find, keep, tidy};

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
    // looking at — and the note is what names the file, so it is never the part
    // that goes missing.
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
    assert!(said.contains("/tmp/shot.jpg"), "{said}");
}

#[test]
fn a_sent_picture_is_read_back_as_a_picture_and_the_words_about_it() {
    // ⚠ The round trip is the whole feature. What the runner writes to the CLI
    // is what a reader meets again in the transcript, and until this the reader
    // met a sentence about a file path — so the one person who could not see the
    // screenshot was the one who took it.
    let line = console::protocol::prompt_with_image(
        "what is wrong with this?",
        "image/png",
        "AAAA",
        std::path::Path::new("/home/example/.console/images/s1/2026-08-05-184700Z.png"),
    );

    let read = console::protocol::read_recorded(&line);

    assert_eq!(
        read,
        vec![
            console::protocol::Event::Shown {
                name: "2026-08-05-184700Z.png".to_string()
            },
            console::protocol::Event::Prompt {
                text: "what is wrong with this?".to_string()
            },
        ],
        "the picture first, then the words, and the path is not among them"
    );
}

#[test]
fn a_picture_sent_wordlessly_reads_back_as_just_the_picture() {
    // Its note is addressed to the session, not to the reader, who is looking
    // straight at the thing it describes. A bubble saying where the file is kept
    // beside the file itself is furniture.
    let line = console::protocol::prompt_with_image(
        "",
        "image/png",
        "AAAA",
        std::path::Path::new("/home/example/.console/images/s1/2026-08-05-184700Z.png"),
    );

    assert_eq!(
        console::protocol::read_recorded(&line),
        vec![console::protocol::Event::Shown {
            name: "2026-08-05-184700Z.png".to_string()
        }]
    );
}

#[test]
fn words_that_merely_talk_about_an_image_are_left_whole() {
    // The note is only read out of a message that actually carries a picture, so
    // an ordinary sentence keeps every word of itself however it is phrased.
    let said = "the image is also at the bottom of the page, oddly";
    let line = console::protocol::prompt(said);

    assert_eq!(
        console::protocol::read_recorded(&line),
        vec![console::protocol::Event::Prompt {
            text: said.to_string()
        }]
    );
}

#[test]
fn a_kept_picture_can_be_read_back_by_name() {
    // What the transcript's thumbnail is fetched with.
    let root = scratch("find");
    let held = keep(&root, "s1", "image/png", PNG, "2026-08-05-184700Z").expect("kept");
    let name = held.path.file_name().expect("name").to_string_lossy();

    let (bytes, media_type) = find(&root, "s1", &name).expect("found");

    assert_eq!(bytes, PNG);
    assert_eq!(media_type, "image/png", "sniffed, not taken off the name");
}

#[test]
fn a_name_that_climbs_out_of_the_directory_is_not_served() {
    // ⚠ Both halves arrive off a URL, and this one hands the bytes back. Without
    // the whitelist it is a file reader with the console's own permissions.
    let root = scratch("climb");
    keep(&root, "s1", "image/png", PNG, "2026-08-05-184700Z").expect("kept");

    assert!(find(&root, "s1", "../s1/2026-08-05-184700Z.png").is_none());
    assert!(find(&root, "..", "2026-08-05-184700Z.png").is_none());
}

#[test]
fn a_file_that_is_not_a_picture_is_not_served_as_one() {
    // Nothing else writes into these directories, but the media type is what a
    // browser is told to trust, and it is cheaper to sniff than to reason about
    // who could have put a file there.
    let root = scratch("not-a-picture");
    std::fs::create_dir_all(root.join("s1")).expect("dir");
    std::fs::write(root.join("s1").join("notes.txt"), b"not an image").expect("write");

    assert!(find(&root, "s1", "notes.txt").is_none());
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

// ---------------------------------------------------------------------------
// The other direction: a picture a session pointed at.
//
// These fetch over loopback from a server the test starts, rather than stubbing
// the client. What is being tested is what this does with an answer — a lying
// `Content-Type`, an error page that came back 200, a body that never ends — and
// none of those exist except on the wire.
// ---------------------------------------------------------------------------

use console::images::{REACH, Reason, fetch};

/// A server that answers everything with the same thing. Returns its base URL.
async fn answering(status: u16, kind: &'static str, body: Vec<u8>) -> String {
    let body = std::sync::Arc::new(body);
    let app = axum::Router::new().fallback(move || {
        let body = body.clone();
        async move {
            (
                axum::http::StatusCode::from_u16(status).expect("a status"),
                [(axum::http::header::CONTENT_TYPE, kind)],
                body.to_vec(),
            )
        }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let at = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{at}/whatever.png")
}

/// A server that answers in chunks and never says how many — so the size guard
/// has to hold without a `Content-Length` to read.
async fn dribbling(pieces: usize, each: usize) -> String {
    let app = axum::Router::new().fallback(move || async move {
        let stream = tokio_stream::iter(
            (0..pieces).map(move |_| Ok::<_, std::io::Error>(vec![b'\xff'; each])),
        );
        axum::body::Body::from_stream(stream)
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let at = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{at}/endless.png")
}

#[tokio::test]
async fn a_picture_from_elsewhere_is_what_its_bytes_say_and_not_what_the_server_says() {
    // The server observe runs is `python3 -m http.server`, which types a file by
    // its extension — so the claim is a guess about a name, and the console
    // hands what comes back to an `<img>` under the type it declares. Sniffed
    // here for the same reason it is sniffed on the way in.
    let at = answering(200, "text/plain", PNG.to_vec()).await;

    let got = fetch(&at).await.expect("fetched");

    assert_eq!(got.media_type, "image/png", "the bytes win");
    assert_eq!(got.bytes, PNG);
}

#[tokio::test]
async fn an_error_page_that_came_back_200_is_refused_with_its_own_first_line() {
    // ⚠ **The case a status check alone would pass.** A server that lost the
    // file, a proxy that wants a login, a directory listing — all of them are a
    // successful HTTP response, and all of them would reach the phone as a
    // picture that will not draw. The first line is what tells them apart, and
    // it is the only thing the person who tapped the link can act on.
    let at = answering(
        200,
        "text/html",
        b"<!DOCTYPE html><title>404</title>".to_vec(),
    )
    .await;

    let why = fetch(&at).await.expect_err("refused");

    assert!(matches!(why, Reason::Answered(_)), "the far end's fault");
    let said = why.to_string();
    assert!(said.contains("not a PNG"), "{said}");
    assert!(
        said.contains("<!DOCTYPE html>"),
        "quotes what it got: {said}"
    );
}

#[tokio::test]
async fn a_status_that_is_not_success_is_carried_back_as_that_status() {
    let at = answering(404, "text/plain", b"no such render".to_vec()).await;

    let why = fetch(&at).await.expect_err("refused");

    assert_eq!(why.to_string(), "it answered 404 Not Found");
}

#[tokio::test]
async fn a_far_end_that_is_not_listening_is_a_sentence_rather_than_a_wait() {
    // The ordinary way this fails: the session that rendered the picture has
    // stopped its server, and the link outlives it in the transcript. A phone
    // showing a spinner for that would be indistinguishable from a slow one.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let at = listener.local_addr().expect("an address");
    drop(listener);

    let why = fetch(&format!("http://{at}/gone.png"))
        .await
        .expect_err("refused");

    assert!(matches!(why, Reason::Answered(_)));
    assert!(why.to_string().starts_with("could not reach it"), "{why}");
}

#[tokio::test]
async fn a_scheme_this_does_not_fetch_is_refused_before_anything_is_asked() {
    // ⚠ **This used to include `file:`, on an argument that was already false
    // when it was written.** It said refusing the scheme was what stopped
    // `~/.ssh/id_ed25519` being served to whoever tapped a link — but the `/`
    // arm at the top of [`fetch`] has always read any local path, so the same
    // key was reachable by writing it without the scheme. The refusal guarded
    // nothing and cost `coach` three dead picture links (memview#1373).
    //
    // What actually guards it is the sniff, and it is tested where it lives:
    // `a_file_that_is_not_a_picture_is_not_served_as_one_from_disk_either` and
    // `a_file_url_gets_the_same_sniff_as_a_bare_path`.
    //
    // `Asked` is the proof nothing was fetched — that arm returns before the
    // client is built.
    for url in ["ftp://somewhere/x.png", "not a url"] {
        let why = fetch(url).await.expect_err("refused");
        assert!(matches!(why, Reason::Asked(_)), "{url} was fetched");
    }
}

#[tokio::test]
async fn something_too_large_for_the_wire_is_refused_on_the_servers_own_claim() {
    // The cheap arm: an honest `Content-Length` is refused before the body is
    // read at all.
    let mut oversize = PNG.to_vec();
    oversize.resize(REACH + 1, 0);
    let at = answering(200, "image/png", oversize).await;

    let why = fetch(&at).await.expect_err("refused");

    // ⚠ **The sentence, not just the refusal.** Both guards refuse this body;
    // only the wording says which one did, so an assertion on "too large" would
    // pass with the early arm deleted and this test would be testing the one
    // below twice.
    assert_eq!(
        why.to_string(),
        "it says it is 8 MB, and this fetches at most 8 MB"
    );
}

#[tokio::test]
async fn something_too_large_is_refused_while_it_arrives_when_nothing_declared_it() {
    // ⚠ **The arm above cannot cover this one.** A `Content-Length` is the
    // server's claim about itself; a chunked answer makes no claim, and this is
    // the guard that decides how much of one this process will hold. Refused
    // part way rather than after, so the memory is bounded by [`REACH`] and not
    // by how long the far end feels like writing.
    let at = dribbling(REACH / (64 * 1024) + 2, 64 * 1024).await;

    let why = fetch(&at).await.expect_err("refused");

    assert_eq!(
        why.to_string(),
        "it went past the 8 MB this fetches, without ever saying how big it was"
    );
}

// ---------------------------------------------------------------------------
// The same route, given a place on this disk rather than an address.
//
// ⚠ **The shape a session actually writes.** It has the file it just rendered;
// the URL exists only if it is also running a server. observe wrote
// `![Photo: cabinet corner](/Users/…/lroom-at20s-photo-upright.jpg)`.
// ---------------------------------------------------------------------------

/// A file with these bytes in it, under a directory this test owns.
fn on_disk(name: &str, bytes: &[u8]) -> std::path::PathBuf {
    let dir = scratch(name);
    std::fs::create_dir_all(&dir).expect("a directory");
    let path = dir.join("render.png");
    std::fs::write(&path, bytes).expect("written");
    path
}

#[tokio::test]
async fn a_picture_named_by_its_place_on_this_disk_is_read_from_it() {
    let path = on_disk("read", JPEG);

    let got = fetch(path.to_str().expect("a path")).await.expect("read");

    // Sniffed here as everywhere: the name says `.png` and the bytes say JPEG,
    // and what the phone is told has to be what it is about to decode.
    assert_eq!(got.media_type, "image/jpeg");
    assert_eq!(got.bytes, JPEG);
}

#[tokio::test]
async fn a_file_that_is_not_a_picture_is_not_served_as_one_from_disk_either() {
    // ⚠ **The bound on what this route can hand out.** It will open any path,
    // which is deliberate (see `from_disk`) — what stops it being a way to read
    // a key or a transcript is that nothing which fails the sniff comes back.
    let path = on_disk("secret", b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 pippijn@mac\n");

    let why = fetch(path.to_str().expect("a path"))
        .await
        .expect_err("refused");

    assert!(matches!(why, Reason::Answered(_)));
    let said = why.to_string();
    assert!(said.contains("not a PNG"), "{said}");
    // And the refusal quotes the head of it, which is how an error page names
    // itself — so a private file's first line must not be in there.
    assert!(
        !said.contains("ssh-ed25519"),
        "the refusal quoted the file: {said}"
    );
}

/// ⚠ **`file:` was refused, and the refusal was recorded as proof the bound
/// held.** It was only ever put to a scheme nobody writes. `coach` writes
/// `[caption](file:///Volumes/…/soft_squat_left.png)` in ordinary prose — three
/// on 2026-09-03, every one dead, while the identical path without the scheme
/// served 200 image/png. A refusal tested only against a hostile shape looks
/// right until something friendly is put to it (memview#1373).
#[tokio::test]
async fn a_file_url_is_the_path_it_names() {
    let path = on_disk("scheme", PNG);
    let url = format!("file://{}", path.to_str().expect("a path"));

    let got = fetch(&url).await.expect("read");

    assert_eq!(got.media_type, "image/png");
    assert_eq!(got.bytes, PNG);
}

/// ⚠ **Percent-decoded, because a `file:` URL is encoded and a disk path is
/// not.** Reading the raw text would hand the filesystem `soft%20squat.png`,
/// which names nothing — and the render server writes names with spaces.
#[tokio::test]
async fn a_file_url_is_decoded_before_the_disk_sees_it() {
    let path = on_disk("soft squat", PNG);
    let url = reqwest::Url::from_file_path(&path).expect("an absolute path");

    let got = fetch(url.as_str()).await.expect("read");

    assert!(
        url.as_str().contains("%20"),
        "the URL should be encoded: {url}"
    );
    assert_eq!(got.bytes, PNG);
}

/// ⚠ **A host is another machine, and this console must not answer for it.**
/// `file://elsewhere/render.png` names a file on `elsewhere`; reading it off
/// THIS disk would be the mistake `shell_ops::resolve` refuses for `host:path`.
/// Refused before anything is opened, so it is an `Asked` and a 400.
#[tokio::test]
async fn a_file_url_naming_another_machine_is_not_this_disk() {
    let why = fetch("file://elsewhere/render.png")
        .await
        .expect_err("refused");

    assert!(matches!(why, Reason::Asked(_)), "{why}");
    assert!(why.to_string().contains("this machine"), "{why}");
}

/// The sniff is the bound, and it does not care how the path was spelled.
#[tokio::test]
async fn a_file_url_gets_the_same_sniff_as_a_bare_path() {
    let path = on_disk(
        "keyfile",
        b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5 someone@example\n",
    );
    let url = format!("file://{}", path.to_str().expect("a path"));

    let why = fetch(&url).await.expect_err("refused");

    assert!(why.to_string().contains("not a PNG"), "{why}");
    assert!(
        !why.to_string().contains("ssh-ed25519"),
        "the refusal quoted the file: {why}"
    );
}

/// A scheme that is neither a fetch nor a path still says so.
#[tokio::test]
async fn a_scheme_this_does_not_speak_is_still_refused() {
    let why = fetch("mailto:someone@example.invalid")
        .await
        .expect_err("refused");

    assert!(matches!(why, Reason::Asked(_)), "{why}");
}

#[tokio::test]
async fn a_path_that_is_not_there_says_so_rather_than_hanging() {
    // The everyday one: the render was overwritten by the next run, or the
    // directory was cleaned.
    let why = fetch("/no/such/render.png").await.expect_err("refused");

    assert!(why.to_string().starts_with("/no/such/render.png:"), "{why}");
}

#[tokio::test]
async fn a_directory_is_not_a_picture() {
    let dir = scratch("dir");
    std::fs::create_dir_all(&dir).expect("a directory");

    let why = fetch(dir.to_str().expect("a path"))
        .await
        .expect_err("refused");

    assert!(why.to_string().ends_with("is not a file"), "{why}");
}

#[tokio::test]
async fn a_file_too_large_for_the_wire_is_refused_at_its_size_not_after_reading_it() {
    let mut heavy = PNG.to_vec();
    heavy.resize(REACH + 1, 0);
    let path = on_disk("heavy", &heavy);

    let why = fetch(path.to_str().expect("a path"))
        .await
        .expect_err("refused");

    assert_eq!(why.to_string(), "it is 8 MB, and this serves at most 8 MB");
}
