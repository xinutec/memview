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

use std::path::{Path, PathBuf};

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
