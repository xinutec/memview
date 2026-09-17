//! What the console is allowed to do, and where it listens.
//!
//! The default bind is loopback: the console can start agents holding this
//! machine's credentials, and a process already running as this user gains nothing
//! by asking. Anywhere else waits for the client-certificate gate — see
//! `docs/agent-console.md`.

use std::path::{Path, PathBuf};

use crate::session::Spawn;

/// Where the gate's own material comes from. Absent means no TLS, which means
/// loopback only.
#[derive(Debug, Clone)]
pub struct Tls {
    pub cert_file: String,
    pub key_file: String,
    /// SHA-256 of each client key's SubjectPublicKeyInfo, as hex.
    pub pins: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub bind: String,
    /// A plaintext listener for this machine only, used only when the gate is on: the
    /// gated socket demands a certificate, and the desk reaches a headless Mac through
    /// an SSH forward that has none. A second port because a wildcard bind already
    /// covers loopback.
    pub desk: String,
    /// Set only when a certificate, a key and at least one pin are all present.
    pub tls: Option<Tls>,
    /// Directories a session may be started in, and their subdirectories.
    pub dirs: Vec<PathBuf>,
    pub spawn: Spawn,
    pub static_dir: Option<String>,
    /// Where to read the account's rate-limit figure from. See [`crate::usage`].
    pub usage_url: Option<String>,
    /// Where the one-sentence summaries are kept between runs. See
    /// [`crate::gist`].
    pub gists: PathBuf,
    /// Where each conversation's permission mode is kept between runs. See
    /// [`crate::modes`] — nothing else on disk records it.
    pub modes: PathBuf,
    /// Where unsent words are kept, so both devices hold the same draft. See
    /// [`crate::drafts`].
    pub drafts: PathBuf,
}

/// The home dashboard, where the rate-limit figure is published. See [`crate::usage`].
const DEFAULT_USAGE_URL: &str = "https://home.xinutec.org/api/usage";

/// Where sessions may run when nothing says otherwise: the working repositories.
const DEFAULT_DIRS: &str = "Code";

impl Config {
    pub fn from_env() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let dirs = std::env::var("CONSOLE_DIRS")
            .unwrap_or_else(|_| format!("{home}/{DEFAULT_DIRS}"))
            .split(':')
            .filter(|entry| !entry.is_empty())
            .map(PathBuf::from)
            .collect();
        // All three or none: a certificate without pins would serve anybody, and pins
        // without a certificate would look configured while the socket stayed plaintext.
        let tls = match (
            std::env::var("CONSOLE_TLS_CERT"),
            std::env::var("CONSOLE_TLS_KEY"),
            std::env::var("CONSOLE_CLIENT_KEYS"),
        ) {
            (Ok(cert_file), Ok(key_file), Ok(pins)) if !pins.trim().is_empty() => Some(Tls {
                cert_file,
                key_file,
                // The file wins over the variable: an upgrade is an `execve`, which inherits the
                // pin list `console.sh` exported at the ORIGINAL launch, so a key enrolled since
                // would be refused until a full restart.
                pins: pinned_clients(&home)
                    .unwrap_or_else(|| pins.split(',').map(|p| p.trim().to_string()).collect()),
            }),
            _ => None,
        };
        Self {
            bind: std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:8097".to_string()),
            desk: std::env::var("CONSOLE_DESK_ADDR")
                .unwrap_or_else(|_| "127.0.0.1:8096".to_string()),
            tls,
            dirs,
            spawn: Spawn {
                binary: std::env::var("CLAUDE_BIN").unwrap_or_else(|_| "claude".to_string()),
                model: std::env::var("CONSOLE_MODEL")
                    .ok()
                    .filter(|m| !m.is_empty()),
                // Unset means the CLI's default, under which a headless session is refused every
                // tool call that needs permission — see `Spawn::permission_mode`. The console picks
                // neither `acceptEdits` nor `bypassPermissions` on anybody's behalf.
                permission_mode: std::env::var("CONSOLE_PERMISSION_MODE")
                    .ok()
                    .filter(|mode| !mode.is_empty()),
            },
            static_dir: std::env::var("STATIC_DIR").ok(),
            // Defaulted rather than required: it is the same dashboard for every machine.
            // Empty means a front page with no usage on it.
            usage_url: Some(
                std::env::var("CONSOLE_USAGE_URL")
                    .unwrap_or_else(|_| DEFAULT_USAGE_URL.to_string()),
            )
            .filter(|url| !url.is_empty()),
            // A file, so an upgrade does not pay a model again for sentences it already had.
            // See [`crate::gist`].
            gists: PathBuf::from(std::env::var("CONSOLE_HOME").unwrap_or_else(|_| {
                format!(
                    "{}/.config/agent-console",
                    std::env::var("HOME").unwrap_or_default()
                )
            }))
            .join("gists.json"),
            // The only record anywhere of what a session was allowed to do; lose it and a
            // conversation resumes Manual. See [`crate::modes`].
            modes: PathBuf::from(std::env::var("CONSOLE_HOME").unwrap_or_else(|_| {
                format!(
                    "{}/.config/agent-console",
                    std::env::var("HOME").unwrap_or_default()
                )
            }))
            .join("modes.json"),
            // Words a person wrote and has not sent. See [`crate::drafts`].
            drafts: PathBuf::from(std::env::var("CONSOLE_HOME").unwrap_or_else(|_| {
                format!(
                    "{}/.config/agent-console",
                    std::env::var("HOME").unwrap_or_default()
                )
            }))
            .join("drafts.json"),
        }
    }

    /// The repositories a session could sensibly be started in: one level down from
    /// each allowed directory, keeping what has a `.git`. A convenience for the picker,
    /// not a restriction — `resolve` admits any subdirectory of an allowed one.
    pub fn repos(&self) -> Vec<String> {
        let mut found: Vec<String> = self
            .dirs
            .iter()
            .filter_map(|dir| std::fs::read_dir(dir).ok())
            .flatten()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.path().join(".git").exists())
            .map(|entry| entry.path().display().to_string())
            .collect();
        found.sort();
        found
    }

    /// Resolve a requested directory, or say why it is refused.
    ///
    /// Symlinks are resolved on both sides first, or one inside an allowed directory
    /// pointing out of it walks past a prefix check. A guard rail against mistakes,
    /// not a boundary: a session that starts can reach the whole disk.
    pub fn resolve(&self, requested: &str) -> Result<PathBuf, String> {
        let asked = Path::new(requested);
        let real = asked
            .canonicalize()
            .map_err(|err| format!("{requested}: {err}"))?;
        if !real.is_dir() {
            return Err(format!("{requested} is not a directory"));
        }
        for allowed in &self.dirs {
            let root = allowed.canonicalize().unwrap_or_else(|_| allowed.clone());
            if real == root || real.starts_with(&root) {
                return Ok(real);
            }
        }
        Err(format!("{requested} is not inside an allowed directory"))
    }
}

/// The pinned client keys, read from the file a person edits: one pin per line,
/// `#` comments and blank lines ignored — what `scripts/console.sh` parses and
/// `scripts/enrol.sh` appends to. `None` when there is no such file, leaving the
/// variable in charge; and `None` for an EMPTY file too, since trusting nobody looks
/// exactly like a wrong certificate and is far more likely to be a mistake.
fn pinned_clients(home: &str) -> Option<Vec<String>> {
    let dir =
        std::env::var("CONSOLE_HOME").unwrap_or_else(|_| format!("{home}/.config/agent-console"));
    let text = std::fs::read_to_string(PathBuf::from(dir).join("clients")).ok()?;
    let pins: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_string)
        .collect();
    (!pins.is_empty()).then_some(pins)
}
