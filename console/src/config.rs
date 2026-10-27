//! What the console is allowed to do, and where it listens.
//!
//! Loopback by default: anything that reaches the socket runs code as this user.
//! Anywhere else needs the client-certificate gate.

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
    /// A plaintext loopback listener beside the gated one, for an SSH forward that
    /// has no certificate.
    pub desk: String,
    /// Set only when a certificate, a key and at least one pin are all present.
    pub tls: Option<Tls>,
    /// Directories a session may be started in, and their subdirectories.
    pub dirs: Vec<PathBuf>,
    pub spawn: Spawn,
    pub static_dir: Option<String>,
    /// Where to read the account's rate-limit figure from. See [`crate::usage`].
    pub usage_url: Option<String>,
    /// Where the one-sentence summaries are kept — [`crate::gist`].
    pub gists: PathBuf,
    /// Where each conversation's permission mode is kept — [`crate::modes`].
    pub modes: PathBuf,
    /// Where unsent words are kept — [`crate::drafts`].
    pub drafts: PathBuf,
    /// The phone's way in through isis, with the gate only: without it the console
    /// listens on loopback and there is nothing to reach. See [`crate::tunnel`].
    pub tunnel: Option<crate::tunnel::Dial>,
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
        // All three or none: either half alone looks secure and is not.
        let tls = match (
            std::env::var("CONSOLE_TLS_CERT"),
            std::env::var("CONSOLE_TLS_KEY"),
            std::env::var("CONSOLE_CLIENT_KEYS"),
        ) {
            (Ok(cert_file), Ok(key_file), Ok(pins)) if !pins.trim().is_empty() => Some(Tls {
                cert_file,
                key_file,
                // The file wins: an upgrade inherits the variable from the first launch.
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
                // Unset is the CLI's default; the console grants nothing on anyone's behalf.
                permission_mode: std::env::var("CONSOLE_PERMISSION_MODE")
                    .ok()
                    .filter(|mode| !mode.is_empty())
                    .map(|mode| crate::modes::Mode::named(&mode)),
                // Per conversation, not per console: a resume fills it from the transcript.
                name: None,
            },
            static_dir: std::env::var("STATIC_DIR").ok(),
            // The same dashboard for every machine; empty means no usage shown.
            usage_url: Some(
                std::env::var("CONSOLE_USAGE_URL")
                    .unwrap_or_else(|_| DEFAULT_USAGE_URL.to_string()),
            )
            .filter(|url| !url.is_empty()),
            // A file, so an upgrade does not pay for the sentences again.
            gists: PathBuf::from(std::env::var("CONSOLE_HOME").unwrap_or_else(|_| {
                format!(
                    "{}/.config/agent-console",
                    std::env::var("HOME").unwrap_or_default()
                )
            }))
            .join("gists.json"),
            // The only record of each session's mode.
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
            tunnel: None,
        }
        .with_tunnel()
    }

    /// The way in, once the gate is configured.
    fn with_tunnel(mut self) -> Self {
        if self.tls.is_some() {
            let dir = self
                .drafts
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_default();
            self.tunnel = Some(crate::tunnel::Dial::from_env(&self.bind, &dir));
        }
        self
    }

    /// The repositories one level under each allowed directory, for the picker.
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

    /// Resolve a requested directory, symlinks included, or say why it is refused. A
    /// rail against mistakes, not a boundary.
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

/// The pinned client keys from the pin file, one per line. `None` when the file is
/// absent or empty — an empty one is likelier a mistake than intent.
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
