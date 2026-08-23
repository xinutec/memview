//! Runtime configuration from the environment.
//!
//! Auth is *inert unless configured* (the recall pattern): the Nextcloud
//! login wall and share tokens only activate when SESSION_SECRET +
//! NC_CLIENT_ID + NC_CLIENT_SECRET are all set. Local dev on the Mac serves
//! the corpus open on the LAN; only the isis deployment raises the wall.

use anyhow::{Context, Result};

#[derive(Clone, Debug)]
pub struct Config {
    /// Directory holding the memory corpus: MEMORY.md + one file per memory.
    pub memory_dir: String,
    /// Address to bind the HTTP server to.
    pub bind_addr: String,
    /// JSON file persisting the public share token (no DB in this app).
    pub share_state_file: String,
    /// Base URL used when composing a share link for display.
    pub public_base_url: Option<String>,

    /// Nextcloud OAuth2 (identity-only). None → auth disabled.
    pub auth: Option<AuthConfig>,

    /// Directory of the built Angular bundle to serve (SPA fallback). Unset →
    /// API-only (dev, where `ng serve` proxies).
    pub static_dir: Option<String>,

    /// The co-use artefact, if one has been mined.
    ///
    /// Optional everywhere: it is derived from the session transcripts, which
    /// exist only on the Mac, and the graph is worth serving without it. Never
    /// inside `memory_dir` — the sync replaces that directory wholesale, and
    /// this is not a memory.
    pub couse_file: Option<String>,
    /// Path to `agents.json`. Absent on any machine without the transcripts.
    pub agents_file: Option<String>,
    /// The timeline artefact, beside the roster. Optional like the rest of the
    /// mining: a fresh checkout has none and every page must still work.
    pub doing_file: Option<String>,
    /// The evidence under the timeline. Optional for the same reason, and the
    /// largest of them — a deployment may deliberately not carry it.
    pub effects_file: Option<String>,
    /// The corpus survey: how much of the fleet's shell the reader understands,
    /// and what it did. Optional like the rest, and the SMALLEST of them at
    /// ~7 kB — mined rather than computed because the survey takes 13 seconds.
    pub reading_file: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AuthConfig {
    /// HMAC key for signing session cookies and OAuth state.
    pub session_secret: String,
    /// Base URL of the Nextcloud instance as the *browser* reaches it, no
    /// trailing slash.
    pub nc_base_url: String,
    /// Server-side base URL for token/userinfo calls (cluster-internal
    /// Service DNS on isis, where the pod can't hairpin to the public IP).
    /// Requests here carry a Host header for `nc_base_url`'s host. None →
    /// server-side calls also use `nc_base_url`.
    pub nc_internal_url: Option<String>,
    /// OAuth2 client registered in NC admin (identity flow).
    pub nc_client_id: String,
    pub nc_client_secret: String,
    /// Must match the redirect URI registered for the OAuth2 client.
    pub nc_redirect_uri: String,
    /// Nextcloud user ids permitted to log in. The corpus holds private and
    /// medical context and the host is on a shared VPN, so access is
    /// fail-closed: an empty list (or a user not on it) is rejected. Set via
    /// ALLOWED_USERS (comma-separated).
    pub allowed_users: Vec<String>,
}

fn env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("missing required env var {key}"))
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let auth = match std::env::var("SESSION_SECRET") {
            Ok(session_secret) => {
                let allowed_users = env("ALLOWED_USERS")?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>();
                Some(AuthConfig {
                    session_secret,
                    nc_base_url: env("NC_BASE_URL")?.trim_end_matches('/').to_string(),
                    nc_internal_url: std::env::var("NC_INTERNAL_URL")
                        .ok()
                        .map(|u| u.trim_end_matches('/').to_string()),
                    nc_client_id: env("NC_CLIENT_ID")?,
                    nc_client_secret: env("NC_CLIENT_SECRET")?,
                    nc_redirect_uri: env("NC_REDIRECT_URI")?,
                    allowed_users,
                })
            }
            Err(_) => None,
        };

        Ok(Self {
            memory_dir: env("MEMORY_DIR")?,
            bind_addr: env_or("BIND_ADDR", "0.0.0.0:8091"),
            share_state_file: env_or("SHARE_STATE", "share-state.json"),
            public_base_url: std::env::var("PUBLIC_BASE_URL")
                .ok()
                .map(|u| u.trim_end_matches('/').to_string()),
            auth,
            static_dir: std::env::var("STATIC_DIR").ok(),
            couse_file: std::env::var("COUSE_FILE").ok(),
            agents_file: std::env::var("AGENTS_FILE").ok(),
            doing_file: std::env::var("DOING_FILE").ok(),
            effects_file: std::env::var("EFFECTS_FILE").ok(),
            reading_file: std::env::var("READING_FILE").ok(),
        })
    }

    /// Whether a Nextcloud user id is permitted to use the app.
    pub fn is_allowed(&self, user_id: &str) -> bool {
        match &self.auth {
            Some(a) => a.allowed_users.iter().any(|u| u == user_id),
            None => true,
        }
    }
}
