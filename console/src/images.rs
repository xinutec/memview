//! Pictures sent to a session from the phone, and pictures a session points at.
//!
//! A user message may carry an `image` block beside its text and the CLI forwards
//! it. A copy is kept on disk anyway: the conversation holds the image only until
//! it is compacted away. Kept exactly as long as the conversation — see [`tidy`].
//!
//! The other direction: a session names a rendered thing by an address on this
//! machine's LAN or by a path on this disk, and the phone can reach neither.
//! [`fetch`] serves both from the one place that can.
//!
//! [`sniff`] decides whether bytes are a picture — for what arrives from the
//! phone, what a server answers and what is read off the disk. None of the three
//! believes what it is told.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the copies go. Overridable because this WRITES, and a test has no home
/// directory worth writing to.
pub fn images_root() -> PathBuf {
    if let Ok(set) = std::env::var("CONSOLE_IMAGE_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".console")
        .join("images")
}

/// What the API will take in one image: Anthropic's own 5 MB limit, the backstop
/// for a client that did not scale first. Refused with a reason rather than
/// truncated — half a JPEG is not a smaller picture.
pub const LIMIT: usize = 5 * 1024 * 1024;

/// The formats the API accepts, each with the bytes it begins with. Sniffed, never
/// trusted: a mislabelled file would fail the turn minutes later in another process.
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
/// Named by the moment rather than a counter, since a person looks for the picture
/// they sent this afternoon; a collision within the second takes a suffix.
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
    // Both halves of the name are checked before either joins a path: `Path::join` on
    // a segment holding `..` walks out of the directory silently, and this is the one
    // place in the console that writes a file named from outside.
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

/// Read one kept picture back.
///
/// Both halves of the name are checked here too, for the stronger reason: without
/// the whitelist, `..%2f..%2f.ssh%2fid_ed25519` is a file this would serve. The
/// media type is sniffed, like everywhere else in this module.
pub fn find(root: &Path, session: &str, name: &str) -> Option<(Vec<u8>, &'static str)> {
    if !plain(session) || !plain(name) {
        return None;
    }
    let bytes = std::fs::read(root.join(session).join(name)).ok()?;
    let (media_type, _) = sniff(&bytes)?;
    Some((bytes, media_type))
}

/// The most a picture from somewhere else may weigh. Not [`LIMIT`] — nothing
/// fetched here goes to a model; the bound is the wire to a phone on cellular, and
/// what one request can make this process hold. Counted while reading, since a
/// `Content-Length` is only the server's claim.
pub const REACH: usize = 8 * 1024 * 1024;

/// How long a picture from somewhere else has to arrive: somebody tapped a link
/// and is watching the space where it goes.
const PATIENCE: Duration = Duration::from_secs(10);

/// A picture from somewhere else, and what it turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Sniffed from the bytes, never the `Content-Type` that came with them.
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// Why a picture from somewhere else is not on its way back. Two cases because
/// they blame different parties: refusing to go is a 400 against the asker, a far
/// end that failed is a 502 about somewhere else.
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
/// Both shapes are unreachable from the phone: a URL names an address the one-way
/// VPN routes nothing back to, a path names a file on the Mac. See [`from_disk`]
/// for what the path half may open.
///
/// An open fetch is not a new privilege — a session already runs shell here — but
/// the sniff restrains the RESPONSE: only PNG, JPEG, GIF or WebP come back, so this
/// cannot proxy somebody else's HTML onto the console's origin. SVG is excluded
/// for the same reason; it carries script.
///
/// Nothing is written to disk.
pub async fn fetch(url: &str) -> Result<Fetched, Reason> {
    // A path is the commoner shape: a session writing about what it just rendered
    // has the FILE, and a bare path resolves against the console's own origin, where
    // it falls through to the single-page app.
    if url.starts_with('/') {
        return from_disk(std::path::Path::new(url));
    }
    let asked = reqwest::Url::parse(url).map_err(|why| Reason::Asked(format!("{url}: {why}")))?;
    // `file:` is the path shape wearing a scheme; `coach` writes
    // `[caption](file:///Volumes/…/x.png)` in ordinary prose. It grants nothing the
    // bare path did not: same [`from_disk`], same sniff, same refusal.
    if asked.scheme() == "file" {
        // `to_file_path`, never `asked.path()`: a `file:` URL is percent-encoded, and a
        // non-empty host names another machine — the same mistake `shell_ops` refuses
        // for `host:path`.
        let here = asked
            .to_file_path()
            .map_err(|()| Reason::Asked(format!("{url} does not name a file on this machine")))?;
        return from_disk(&here);
    }
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
    // The claim, refused before a byte is read. The count below is what actually
    // holds; this saves the download when the far end is honest.
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
            // The head of an error page is usually all a person needs: a 200 carrying an
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
/// This will hand out any file on the Mac that IS a picture, and that is
/// deliberate. What bounds it: the sniff (no key or transcript comes back), who
/// can ask (a phone whose TLS terminates here against a pinned key), and who it
/// reaches (the person holding that phone could open the file anyway, and a
/// session that could plant a path already runs shell here). Restricting to the
/// session's working directory was rejected: sessions render into `/tmp`
/// constantly, and a `cp` defeats it regardless.
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
        // Not a word of what is in it — unlike the fetched half, which quotes an error
        // page's first line. A refusal quoting the head would read the first eighty bytes
        // of ANY file on the Mac. The test hands it an ssh key.
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

/// A few words about bytes that are not a picture: an HTML error page or a
/// directory listing, which both say what happened in their first line. Anything
/// not printable ASCII is described rather than quoted.
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

/// The one client, kept because a client is a connection pool. The crypto provider
/// is installed here as well as in `main`: a test that reaches this has run no `main`.
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

/// Delete the copies belonging to conversations no longer on disk, and say how
/// many went.
///
/// The only thing in the console that deletes, so it fails closed: an empty `keep`
/// deletes nothing ([`crate::past::transcript_ids`] is empty both when there are
/// no conversations and when it could not read the directory); only names this
/// module could have written ([`plain`]); only directories directly under `root`.
///
/// A conversation that is still there keeps ALL of its pictures: forty
/// screenshots are forty pieces of evidence, dropped when the conversation goes.
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

/// Whether a string is safe as one path segment: letters, digits and the three
/// marks a session id and a timestamp are made of. A whitelist, since the ways to
/// write a traversal are open-ended.
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
