//! Public share token — the health app's share mechanism, minus the DB.
//!
//! One token for the (single) owner gives an unauthenticated recipient
//! read-only access to the whole corpus. Rotation replaces the token so a
//! leaked old link immediately stops working; revoke deletes it. Persisted
//! as a small JSON file (SHARE_STATE) so it survives restarts.

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;

#[derive(Clone, Serialize, Deserialize)]
pub struct ShareState {
    pub token: String,
    pub created_at: DateTime<Utc>,
    pub last_accessed_at: Option<DateTime<Utc>>,
}

/// 32 random bytes → 43 base64url chars: un-guessable, pastable.
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

pub struct ShareStore {
    path: PathBuf,
    state: Mutex<Option<ShareState>>,
}

impl ShareStore {
    pub fn load(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let state = match std::fs::read(&path) {
            Ok(bytes) => Some(
                serde_json::from_slice(&bytes)
                    .with_context(|| format!("parsing share state {}", path.display()))?,
            ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e).context(format!("reading share state {}", path.display())),
        };
        Ok(Self {
            path,
            state: Mutex::new(state),
        })
    }

    pub fn get(&self) -> Option<ShareState> {
        self.state.lock().expect("share state poisoned").clone()
    }

    /// True when `token` matches the active share token. Constant-time, like the
    /// session HMAC check: `==` would short-circuit on the first wrong byte and
    /// let the timing walk the token out one byte at a time.
    pub fn is_valid(&self, token: &str) -> bool {
        self.get()
            .is_some_and(|s| s.token.as_bytes().ct_eq(token.as_bytes()).into())
    }

    /// Create or rotate the token. Any previous one is gone after this call.
    pub fn rotate(&self) -> Result<ShareState> {
        let fresh = ShareState {
            token: generate_token(),
            created_at: Utc::now(),
            last_accessed_at: None,
        };
        let mut guard = self.state.lock().expect("share state poisoned");
        self.persist(&Some(fresh.clone()))?;
        *guard = Some(fresh.clone());
        Ok(fresh)
    }

    pub fn revoke(&self) -> Result<()> {
        let mut guard = self.state.lock().expect("share state poisoned");
        self.persist(&None)?;
        *guard = None;
        Ok(())
    }

    /// Bump last_accessed_at. Best-effort; a failed write must not fail the
    /// read it decorates.
    pub fn touch(&self) {
        let mut guard = self.state.lock().expect("share state poisoned");
        if let Some(state) = guard.as_mut() {
            state.last_accessed_at = Some(Utc::now());
            if let Err(e) = self.persist(&Some(state.clone())) {
                tracing::warn!("share touch failed: {e:#}");
            }
        }
    }

    fn persist(&self, state: &Option<ShareState>) -> Result<()> {
        match state {
            Some(s) => {
                let json = serde_json::to_vec_pretty(s)?;
                std::fs::write(&self.path, json)
                    .with_context(|| format!("writing share state {}", self.path.display()))?;
            }
            None => match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(e).context(format!("removing share state {}", self.path.display()));
                }
            },
        }
        Ok(())
    }
}

/// Compose the public URL that gets sent to the recipient.
pub fn build_share_url(base_url: &str, token: &str) -> String {
    format!("{}/share/{token}", base_url.trim_end_matches('/'))
}
