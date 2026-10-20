//! Pictures sent to a session from the phone, and pictures a session points at.
//!
//! A sent picture is also kept on disk, since compaction drops it from the
//! conversation, for as long as the conversation lasts ([`tidy`]). A picture a
//! session points at, by LAN address or path, is fetched for the phone ([`fetch`]).
//! [`sniff`] decides what is a picture, never trusting what it is told.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Where the copies go; overridable for tests.
pub fn images_root() -> PathBuf {
    if let Ok(set) = std::env::var("CONSOLE_IMAGE_DIR") {
        return PathBuf::from(set);
    }
    PathBuf::from(std::env::var("HOME").unwrap_or_default())
        .join(".console")
        .join("images")
}

/// The API's limit for one image; larger is refused with a reason.
pub const LIMIT: usize = 5 * 1024 * 1024;

/// The formats the API accepts, by the bytes each begins with.
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
    /// Where the copy went, for the conversation to name.
    pub path: PathBuf,
}

/// Keep a copy, named by the moment it was sent, and say what it is — or why it is
/// not an image.
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
    // Both halves checked before joining a path: a `..` segment would walk out.
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

/// Read one kept picture back. Both halves of the name are whitelisted, or
/// `..%2f..%2f.ssh%2fid_ed25519` would be served.
pub fn find(root: &Path, session: &str, name: &str) -> Option<(Vec<u8>, &'static str)> {
    if !plain(session) || !plain(name) {
        return None;
    }
    let bytes = std::fs::read(root.join(session).join(name)).ok()?;
    let (media_type, _) = sniff(&bytes)?;
    Some((bytes, media_type))
}

/// The most a fetched picture may weigh — for a phone on cellular, and this
/// process's memory. Counted while reading.
pub const REACH: usize = 8 * 1024 * 1024;

/// How long a fetched picture has to arrive: somebody is watching for it.
const PATIENCE: Duration = Duration::from_secs(10);

/// A picture from somewhere else, and what it turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    /// Sniffed from the bytes, never the `Content-Type` that came with them.
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// The longest edge of a thumbnail, in pixels: a phone column at its density.
pub const THUMBNAIL: u32 = 480;

/// `got` scaled to fit `longest` pixels, as JPEG; unchanged if it fits or will not decode.
pub fn thumbnail(got: Fetched, longest: u32) -> Fetched {
    let Ok(picture) = image::load_from_memory(&got.bytes) else {
        return got;
    };
    if picture.width().max(picture.height()) <= longest {
        return got;
    }
    let mut bytes = std::io::Cursor::new(Vec::new());
    let small = picture.thumbnail(longest, longest).into_rgb8();
    if small
        .write_to(&mut bytes, image::ImageFormat::Jpeg)
        .is_err()
    {
        return got;
    }
    Fetched {
        media_type: "image/jpeg".to_string(),
        bytes: bytes.into_inner(),
    }
}

/// Why a fetched picture is not coming: the asker's fault (400) or the far end's (502).
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

/// Fetch a picture a session pointed at, by URL or path — neither reachable from
/// the phone. Only PNG, JPEG, GIF or WebP come back, so this cannot put foreign
/// HTML or SVG script on the console's origin. Nothing is written.
pub async fn fetch(url: &str) -> Result<Fetched, Reason> {
    // A path is the commoner shape.
    if url.starts_with('/') {
        return from_disk(std::path::Path::new(url));
    }
    let asked = reqwest::Url::parse(url).map_err(|why| Reason::Asked(format!("{url}: {why}")))?;
    // `file:` is the path shape with a scheme, and grants nothing more.
    if asked.scheme() == "file" {
        // `to_file_path`: it decodes, and refuses a host naming another machine.
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
    // The claimed size, refused early; the count below is what holds.
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
            // An error page's head usually says what happened.
            described(&bytes)
        ))
    })?;
    Ok(Fetched {
        media_type: media_type.to_string(),
        bytes,
    })
}

/// A picture on this disk, anywhere — deliberately: only pictures come back, only
/// to the pinned phone, whose holder could open the file anyway. Size is checked
/// before reading.
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
        // Never quote the head: it would leak the start of any file.
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

/// A few words about bytes that are not a picture; non-ASCII is described, not quoted.
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

/// The one client, a connection pool. Installs the crypto provider, for tests.
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

/// Delete the copies of conversations no longer on disk, and say how many. The
/// console's only delete, so it fails closed: an empty `keep` deletes nothing, and
/// only names this module could have written go.
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

/// Whether a string is safe as one path segment — a whitelist.
// dev-lint: sanitises path — the whitelist above, `..` refused
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
        // RIFF is a container; an AVI begins the same way.
        if media_type == "image/webp" && bytes.get(8..12) != Some(b"WEBP") {
            continue;
        }
        return Some((media_type, extension));
    }
    None
}
