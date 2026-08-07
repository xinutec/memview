//! Shared application state + the short-lived OAuth `state` store (in-memory,
//! per process — fine for a single-pod deployment). Copied from `messages`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::RngCore;

use crate::config::Config;
use crate::share::ShareStore;
use reader::doing::Doing;

const OAUTH_TTL: Duration = Duration::from_secs(600); // 10 minutes

pub struct PendingOauth {
    created: Instant,
    pub return_to: Option<String>,
}

/// A mined artefact held in memory, reloaded when the file beneath it changes.
///
/// **The one artefact that is not re-read per request.** The corpus is, on
/// purpose — a live session writes memories and they must appear at once — and
/// the roster is small enough to follow suit. The timeline is 10 MB of a
/// hundred thousand rows, and parsing that on every request would trade a page
/// nobody notices for a page nobody waits for. The mtime check keeps the
/// liveness that mattered: the file changes once a night, and the next request
/// after it picks it up.
struct Cached {
    at: Option<std::time::SystemTime>,
    doing: Arc<Doing>,
}

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub http: reqwest::Client,
    pub share: Arc<ShareStore>,
    oauth: Arc<Mutex<HashMap<String, PendingOauth>>>,
    timeline: Arc<Mutex<Option<Cached>>>,
}

impl AppState {
    pub fn new(cfg: Config, http: reqwest::Client, share: ShareStore) -> Self {
        Self {
            cfg: Arc::new(cfg),
            http,
            share: Arc::new(share),
            oauth: Arc::new(Mutex::new(HashMap::new())),
            timeline: Arc::new(Mutex::new(None)),
        }
    }

    /// The timeline, from memory unless the file has changed since.
    pub fn doing(&self) -> Arc<Doing> {
        let Some(path) = self.cfg.doing_file.as_deref() else {
            return Arc::new(Doing::default());
        };
        let path = std::path::Path::new(path);
        let at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut held = self.timeline.lock().expect("timeline poisoned");
        if let Some(cached) = held.as_ref()
            && cached.at == at
        {
            return cached.doing.clone();
        }
        let doing = Arc::new(Doing::load(path).unwrap_or_default());
        *held = Some(Cached {
            at,
            doing: doing.clone(),
        });
        doing
    }

    pub fn create_oauth_state(&self, return_to: Option<String>) -> String {
        let mut bytes = [0u8; 24];
        rand::rng().fill_bytes(&mut bytes);
        let state = hex::encode(bytes);
        let mut map = self.oauth.lock().expect("oauth map poisoned");
        map.retain(|_, v| v.created.elapsed() < OAUTH_TTL);
        map.insert(
            state.clone(),
            PendingOauth {
                created: Instant::now(),
                return_to,
            },
        );
        state
    }

    pub fn consume_oauth_state(&self, state: &str) -> Option<PendingOauth> {
        let mut map = self.oauth.lock().expect("oauth map poisoned");
        let entry = map.remove(state)?;
        if entry.created.elapsed() > OAUTH_TTL {
            return None;
        }
        Some(entry)
    }
}
