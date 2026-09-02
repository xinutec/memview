//! Pictures sent to a session from the phone.
//!
//! The console's one inbound path for something that is not text. It exists
//! because the phone is where the screen being talked about *is*: a layout that
//! settles wrongly, a chart that reads oddly, a photograph of a thing on a desk —
//! all of it was previously describable and not showable.
//!
//! ⚠ **Measured against CLI 2.1.221 rather than assumed.** A user message on
//! stream-json stdin may carry an `image` content block beside its text, the CLI
//! forwards it, and the model reads it — tested with a real screenshot before any
//! of this was written. The alternative design, writing the file to disk and
//! sending its path for the session to open, was dropped once that came back with
//! a description of the picture.
//!
//! A copy is kept on disk anyway. The conversation holds the image only until it
//! is compacted away, and the file is what makes it possible to look again — at
//! full size, which is not what was sent. It is kept for exactly as long as that
//! conversation is: see [`tidy`], which is what stops a directory of megabytes
//! outliving every transcript that explains it.
//!
//! ## And the other direction: a picture a session pointed at
//!
//! A session that renders something names it in the conversation two ways, and
//! the phone could follow neither. **An address**, when the session is also
//! running a server — observe puts its reconstruction previews on an ad-hoc HTTP
//! server — names this machine's LAN, which the phone is not on: it reaches the
//! console through a tunnel isis carries, and the one-way VPN means nothing
//! routes back. **A path** is the commoner one, because a session has the file
//! and only has a URL if it is also serving it; a path names something that is on
//! this Mac and nowhere else.
//!
//! [`fetch`] closes both, from the one place that can reach either: the Mac reads
//! it, and the phone asks the console it is already talking to.
//!
//! Every half of this module answers the same question with the same code: are
//! these bytes a picture? [`sniff`] decides for what arrives from the phone, for
//! what a server answers and for what is read off the disk — and none of the
//! three believes what it is told.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the copies go. Overridable for the same reason
/// [`crate::tasks::tasks_root`] is: a test has no home directory worth writing
/// to, and this one *writes*.
pub fn images_root() -> PathBuf {
    if let Ok(set) = std::env::var("CONSOLE_IMAGE_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".console")
        .join("images")
}

/// What the API will take in one image.
///
/// ⚠ **The API's own limit, not a number of ours.** Anthropic refuses an image
/// over 5 MB, and a phone photograph is routinely larger — so the client scales
/// before sending and this is the backstop for a client that did not. Rejected
/// with a reason rather than truncated: half a JPEG is not a smaller picture.
pub const LIMIT: usize = 5 * 1024 * 1024;

/// The formats the API accepts, each with the bytes it begins with.
///
/// ⚠ **Sniffed, never taken on trust.** The media type arrives from the client
/// and is what the CLI is told, so a mislabelled file would be sent to the API as
/// something it is not — and the API's refusal would arrive as an unexplained
/// failed turn, minutes later, in a different process. Cheaper to know here.
const FORMATS: [(&str, &[u8], &str); 4] = [
    ("image/png", b"\x89PNG\r\n\x1a\n", "png"),
    ("image/jpeg", b"\xff\xd8\xff", "jpg"),
    ("image/gif", b"GIF8", "gif"),
    // RIFF....WEBP — the four bytes at offset 8 are checked separately.
    ("image/webp", b"RIFF", "webp"),
];

/// A picture, once it is known to be one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Held {
    /// What it is, in the API's vocabulary — `image/png` and the rest.
    pub media_type: String,
    /// Where the copy went, so the conversation can name it and anything with
    /// file access can open it again after the context has moved on.
    pub path: PathBuf,
}

/// Keep a copy and say what it is, or say why it is not an image.
///
/// The name carries the moment rather than a counter: these are read by a person
/// looking for the picture they sent this afternoon, and `2026-08-05-184700.png`
/// answers that where `7.png` does not. A collision within the same second takes
/// a suffix rather than overwriting — the older picture is somebody's evidence.
pub fn keep(
    root: &Path,
    session: &str,
    media_type: &str,
    bytes: &[u8],
    stamp: &str,
) -> Result<Held, String> {
    if bytes.is_empty() {
        return Err("that image arrived empty".to_string());
    }
    if bytes.len() > LIMIT {
        return Err(format!(
            "that image is {} MB and the API takes {} MB — scale it down first",
            bytes.len() / (1024 * 1024),
            LIMIT / (1024 * 1024)
        ));
    }
    let (sniffed, extension) = sniff(bytes).ok_or_else(|| {
        format!("that is not a PNG, JPEG, GIF or WebP, whatever it says it is ({media_type})")
    })?;
    // ⚠ **Both halves of the name are checked before either is joined to a
    // path.** The session comes off the URL, and while the roster is asked for it
    // first — so it is an id this console holds — nothing about that is this
    // function's to assume. `Path::join` on a segment containing `..` walks out
    // of the directory silently, and this is the one place in the console that
    // writes a file whose name came from outside.
    if !plain(session) {
        return Err(format!("{session} is not a session name"));
    }
    if !plain(stamp) {
        return Err(format!("{stamp} is not a filename"));
    }

    let dir = root.join(session);
    std::fs::create_dir_all(&dir)
        .map_err(|why| format!("could not make {}: {why}", dir.display()))?;
    let mut path = dir.join(format!("{stamp}.{extension}"));
    for again in 1..100 {
        if !path.exists() {
            break;
        }
        path = dir.join(format!("{stamp}-{again}.{extension}"));
    }
    std::fs::write(&path, bytes)
        .map_err(|why| format!("could not write {}: {why}", path.display()))?;
    Ok(Held {
        media_type: sniffed.to_string(),
        path,
    })
}

/// Read one kept picture back, for a reader that wants to see what it sent.
///
/// ⚠ **Both halves of the name are checked here too, and for the stronger
/// reason.** [`keep`] guards a name it is about to write; this guards one it is
/// about to *read and hand out*, and both halves arrive off a URL. Without the
/// whitelist, `..%2f..%2f.ssh%2fid_ed25519` is a file this would happily serve.
///
/// The media type is sniffed rather than taken from the extension, because it is
/// sniffed everywhere else in this module and one place that trusts a file name
/// is the place that will be wrong.
pub fn find(root: &Path, session: &str, name: &str) -> Option<(Vec<u8>, &'static str)> {
    if !plain(session) || !plain(name) {
        return None;
    }
    let bytes = std::fs::read(root.join(session).join(name)).ok()?;
    let (media_type, _) = sniff(&bytes)?;
    Some((bytes, media_type))
}

/// The most a picture from somewhere else may weigh.
///
/// ⚠ **Not [`LIMIT`], because nothing fetched here is sent to a model.** What
/// bounds this is the wire: a render travels the tunnel to a phone that may be
/// on cellular, and something that will not arrive is not worth beginning. It is
/// also the ceiling on what one request can make this process hold, which is why
/// it is counted while reading rather than after — a `Content-Length` is the
/// server's claim about itself, and a server that understates it meets the same
/// number a second time.
pub const REACH: usize = 8 * 1024 * 1024;

/// How long a picture from somewhere else has to arrive.
///
/// Generous next to the two seconds [`crate::tasks`] allows itself: that is a
/// poll behind a page that can go without, and this is somebody who tapped a
/// link and is watching the space where the picture goes.
const PATIENCE: Duration = Duration::from_secs(10);

/// A picture from somewhere else, and what it turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Sniffed from the bytes, never the `Content-Type` that came with them.
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// Why a picture from somewhere else is not on its way back.
///
/// Two cases rather than one string, because they blame different parties and
/// the answer's status code is the only place that distinction survives: this
/// console refusing to go is a 400 against whoever asked, and a far end that
/// failed is a 502 about somewhere else. Both carry a sentence, because the
/// person who tapped the link is the one who has to decide whether to re-render
/// or to start the server again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// Nothing was fetched: this is not a URL this will go to.
    Asked(String),
    /// It was fetched, and what came back is not a picture.
    Answered(String),
}

impl std::fmt::Display for Reason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Reason::Asked(why) | Reason::Answered(why) => f.write_str(why),
        }
    }
}

/// Fetch a picture a session pointed at, by URL or by where it sits on this disk.
///
/// ⚠ **This exists because the phone is not on the LAN, and not on the disk
/// either.** Both shapes a session writes are unreachable from where they are
/// read: a URL names an address the phone cannot route to — it talks to the
/// console down a tunnel this Mac dialled out to isis, and nothing goes back the
/// other way — and a path names a file that is on the Mac and nowhere else.
/// Reading here and serving from the console is the only arrangement that does
/// not amend the one-way VPN. See [`crate::images`]'s module note, and
/// [`from_disk`] for what the path half is allowed to open.
///
/// ## Why an open fetch is not a new privilege
///
/// The URL comes out of a conversation, so in principle a session decides where
/// this goes — and a session already runs shell on this machine, so an allowlist
/// would restrain nobody it was written for. What the sniff below *does*
/// restrain is the response: only bytes that are a PNG, JPEG, GIF or WebP come
/// back, so this cannot be used as a general proxy that puts somebody else's
/// HTML on the console's own origin, which is the one thing here that would be a
/// new privilege. SVG is excluded by the same table for the same reason — an SVG
/// carries script, and the other four cannot.
///
/// Nothing is written to disk. A kept picture is one somebody sent and can look
/// for again ([`keep`]); this is a window onto a file that lives somewhere else
/// and is rewritten by whatever renders it.
pub async fn fetch(url: &str) -> Result<Fetched, Reason> {
    // ⚠ **A path is the commoner shape, not the exotic one.** A session writing
    // about something it just rendered has the FILE; the URL exists only if it
    // also happens to be running a server. So observe wrote
    // `![Photo: cabinet corner](/Users/…/lroom-at20s-photo-upright.jpg)` and it
    // was a link to nowhere — a path resolves against the console's own origin,
    // where it falls through to the single-page app. Reading it here is what
    // makes the ordinary thing to write the thing that works.
    if url.starts_with('/') {
        return from_disk(std::path::Path::new(url));
    }
    let asked = reqwest::Url::parse(url).map_err(|why| Reason::Asked(format!("{url}: {why}")))?;
    if !matches!(asked.scheme(), "http" | "https") {
        return Err(Reason::Asked(format!(
            "{} is not a scheme this fetches — http and https are",
            asked.scheme()
        )));
    }

    let answer = client()
        .get(asked)
        .send()
        .await
        .map_err(|why| Reason::Answered(format!("could not reach it: {why}")))?;
    if !answer.status().is_success() {
        return Err(Reason::Answered(format!("it answered {}", answer.status())));
    }
    // The claim, refused before a byte is read. The check below is what actually
    // holds; this one saves the download when the far end is honest, and it can
    // name the size where the other one can only say it went past.
    if let Some(size) = answer.content_length()
        && size > REACH as u64
    {
        return Err(Reason::Answered(format!(
            "it says it is {} MB, and this fetches at most {} MB",
            size / (1024 * 1024),
            REACH / (1024 * 1024)
        )));
    }

    let mut answer = answer;
    let mut bytes: Vec<u8> = Vec::new();
    while let Some(piece) = answer
        .chunk()
        .await
        .map_err(|why| Reason::Answered(format!("it stopped part way: {why}")))?
    {
        if bytes.len() + piece.len() > REACH {
            return Err(Reason::Answered(format!(
                "it went past the {} MB this fetches, without ever saying how big it was",
                REACH / (1024 * 1024)
            )));
        }
        bytes.extend_from_slice(&piece);
    }

    let (media_type, _) = sniff(&bytes).ok_or_else(|| {
        Reason::Answered(format!(
            "what came back is not a PNG, JPEG, GIF or WebP{}",
            // The head of an error page is worth more than the sentence above,
            // and is usually all of what a person needs: a 200 carrying an
            // apology reads exactly like a broken picture without it.
            described(&bytes)
        ))
    })?;
    Ok(Fetched {
        media_type: media_type.to_string(),
        bytes,
    })
}

/// A picture the session named by where it is on this disk.
///
/// ⚠ **What this will hand out is any file on the Mac that IS a picture**, which
/// is Pippijn's decision (2026-09-02) and belongs in the open rather than in a
/// changelog. Three things bound it. The sniff below: only PNG, JPEG, GIF and
/// WebP come back, so this is not a way to read a key or a transcript. Who can
/// ask: the phone reaches the console over a tunnel whose TLS terminates here
/// against a pinned key. And who it reaches: the person holding that phone could
/// have opened the file anyway, and a session that could plant a path already
/// runs shell on the machine the file is on. The narrower rule considered —
/// only under the session's own working directory — was rejected because
/// sessions render into `/tmp` constantly, and a session can copy a file into
/// its own tree in one command regardless.
///
/// The size is read from the metadata before the bytes, so a video linked by
/// mistake is refused at its size rather than after being loaded.
fn from_disk(path: &std::path::Path) -> Result<Fetched, Reason> {
    let about = std::fs::metadata(path)
        .map_err(|why| Reason::Answered(format!("{}: {why}", path.display())))?;
    if !about.is_file() {
        return Err(Reason::Answered(format!(
            "{} is not a file",
            path.display()
        )));
    }
    if about.len() > REACH as u64 {
        return Err(Reason::Answered(format!(
            "it is {} MB, and this serves at most {} MB",
            about.len() / (1024 * 1024),
            REACH / (1024 * 1024)
        )));
    }
    let bytes = std::fs::read(path)
        .map_err(|why| Reason::Answered(format!("{}: {why}", path.display())))?;
    let (media_type, _) = sniff(&bytes).ok_or_else(|| {
        // ⚠ **Not a word of what is in it**, which is where this differs from the
        // fetched half — that quotes the first line, because an error page
        // naming itself is the whole of what a person needs and a server chose
        // to send it. Here there is no server and no choosing: a refusal that
        // quoted the head would make this route a way to read the first eighty
        // bytes of ANY file on the Mac, and the sniff would have stopped
        // exactly nothing. Caught by the test that hands it an ssh key.
        Reason::Answered(format!(
            "{} is not a PNG, JPEG, GIF or WebP — {} bytes of something else",
            path.display(),
            bytes.len()
        ))
    })?;
    Ok(Fetched {
        media_type: media_type.to_string(),
        bytes,
    })
}

/// A few words about bytes that are not a picture, for the sentence that says so.
///
/// Text only, and short: an HTML error page and a directory listing are the two
/// things this actually meets, and both say what happened in their first line.
/// Anything that is not printable ASCII is described rather than quoted, because
/// a viewer showing a fragment of a binary is a viewer that looks broken itself.
fn described(bytes: &[u8]) -> String {
    let head: Vec<u8> = bytes.iter().copied().take(80).collect();
    if head.is_empty() {
        return " — it answered with nothing at all".to_string();
    }
    if head
        .iter()
        .all(|byte| byte.is_ascii_graphic() || byte.is_ascii_whitespace())
    {
        let text = String::from_utf8_lossy(&head);
        return format!(" — it begins {:?}", text.trim());
    }
    format!(" — {} bytes of something else", bytes.len())
}

/// The one client, kept because a client is a connection pool.
///
/// ⚠ **The crypto provider is installed here as well as in `main`**, for the
/// reason [`crate::tasks::Reader::at`] gives at length: building a TLS client
/// with no process-wide provider panics inside reqwest, and a test that reaches
/// this function has run no `main`.
fn client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder()
            .timeout(PATIENCE)
            .build()
            .unwrap_or_default()
    })
}

/// Delete the copies belonging to conversations that are no longer on disk, and
/// say how many went.
///
/// ⚠ **This deletes files, and it is the only thing in the console that does.**
/// Everything about it is therefore written to fail closed:
///
/// - **An empty `keep` deletes nothing.** [`crate::past::transcript_ids`] returns
///   nothing both when there are no conversations and when it could not read the
///   directory, exactly as the gist store's walk does — but here the cost of
///   reading the second as the first is somebody's pictures rather than a cache
///   that pays a model to refill itself. The true empty case has nothing to tidy.
/// - **Only names this module could have written.** A directory whose name fails
///   [`plain`] is left where it is: it did not come from [`keep`], so whatever it
///   is, it is not ours to remove.
/// - **Only directories, only directly under `root`.** No recursion looking for
///   more to do.
///
/// A picture is kept for as long as its conversation is, which is what makes it
/// possible to look again after the context has moved on. **A conversation that
/// is still there keeps all of its pictures however many it has** — a session
/// that has been shown forty screenshots is not a leak, it is forty pieces of
/// evidence somebody may want, and the moment to drop them is when the
/// conversation itself goes.
pub fn tidy(root: &Path, keep: &std::collections::BTreeSet<String>) -> usize {
    if keep.is_empty() {
        return 0;
    }
    let mut gone = 0;
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|it| it.to_str()) else {
            continue;
        };
        if !plain(name) || keep.contains(name) {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                tracing::info!("images: {name} has no transcript left — dropped its pictures");
                gone += 1;
            }
            Err(why) => tracing::warn!("images: could not drop {}: {why}", path.display()),
        }
    }
    gone
}

/// Whether a string is safe to be one segment of a path: letters, digits and the
/// three punctuation marks a session id and a timestamp are made of, and nothing
/// else. A whitelist rather than a search for `..` and `/` — the ways to write a
/// traversal are open-ended and the shapes this actually needs are not.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|it| it.is_ascii_alphanumeric() || matches!(it, '-' | '_' | '.'))
        && !name.contains("..")
}

/// What these bytes actually are, by their first few.
fn sniff(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    for (media_type, magic, extension) in FORMATS {
        if !bytes.starts_with(magic) {
            continue;
        }
        // RIFF is a container: an AVI begins the same way and is not an image.
        if media_type == "image/webp" && bytes.get(8..12) != Some(b"WEBP") {
            continue;
        }
        return Some((media_type, extension));
    }
    None
}
