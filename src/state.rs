//! Shared application state, and the short-lived OAuth `state` store (in-memory,
//! per process).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;
use std::time::{Duration, Instant};

use rand::RngCore;

use crate::config::Config;
use crate::share::ShareStore;
use reader::doing::Doing;
use reader::effects::Effects;
use reader::reading::CorpusRead;

/// How long a pending sign-in stays valid. Public because the cookie carrying
/// the same sign-in must expire with it.
pub const OAUTH_TTL: Duration = Duration::from_secs(600); // 10 minutes

pub struct PendingOauth {
    created: Instant,
    pub return_to: Option<String>,
}

/// A mined artefact held in memory, reloaded when the file beneath it changes —
/// the one thing not re-read per request: the timeline is 10 MB, and it changes
/// once a night.
struct Cached {
    at: Option<std::time::SystemTime>,
    doing: Arc<Doing>,
}

/// How many effects each `(agent, minute)` has, keyed by the EFFECTS artefact's
/// own agent index.
pub type EffectCounts = Arc<HashMap<(u32, i64), u32>>;

/// The same, for the effects: 35 MB, opened by the tap that is meant to feel instant.
struct CachedEffects {
    at: Option<std::time::SystemTime>,
    effects: Arc<Effects>,
    /// How many effects each `(agent, minute)` has, built once with the artefact:
    /// a page is 200 moments and the artefact 327,852 rows.
    counts: EffectCounts,
}

/// The corpus survey, held the same way for a weaker reason (7 kB): the mtime
/// check is what makes "the artefact changed" the only way a served number moves.
struct CachedReading {
    at: Option<std::time::SystemTime>,
    reading: Option<Arc<CorpusRead>>,
}

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub http: reqwest::Client,
    pub share: Arc<ShareStore>,
    oauth: Arc<Mutex<HashMap<String, PendingOauth>>>,
    timeline: Arc<Mutex<Option<Cached>>>,
    effects: Arc<Mutex<Option<CachedEffects>>>,
    reading: Arc<Mutex<Option<CachedReading>>>,
}

impl AppState {
    pub fn new(cfg: Config, http: reqwest::Client, share: ShareStore) -> Self {
        Self {
            cfg: Arc::new(cfg),
            http,
            share: Arc::new(share),
            oauth: Arc::new(Mutex::new(HashMap::new())),
            timeline: Arc::new(Mutex::new(None)),
            effects: Arc::new(Mutex::new(None)),
            reading: Arc::new(Mutex::new(None)),
        }
    }

    /// The effects, from memory unless the file has changed. Absent config means an
    /// empty artefact, not a 500.
    pub fn effects(&self) -> Arc<Effects> {
        self.effects_and_counts().0
    }

    /// The effects and the per-`(agent, minute)` count, from one cache: they go
    /// stale at the same instant.
    pub fn effects_and_counts(&self) -> (Arc<Effects>, EffectCounts) {
        let Some(path) = self.cfg.effects_file.as_deref() else {
            return (Arc::new(Effects::default()), Arc::new(HashMap::new()));
        };
        let path = std::path::Path::new(path);
        let at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut held = self.effects.lock();
        if let Some(cached) = held.as_ref()
            && cached.at == at
        {
            return (cached.effects.clone(), cached.counts.clone());
        }
        let effects = Arc::new(Effects::load(path).unwrap_or_default());
        let mut counts: HashMap<(u32, i64), u32> = HashMap::new();
        for row in &effects.rows {
            *counts.entry((row.a, row.t)).or_insert(0) += 1;
        }
        let counts = Arc::new(counts);
        *held = Some(CachedEffects {
            at,
            effects: effects.clone(),
            counts: counts.clone(),
        });
        (effects, counts)
    }

    /// The timeline, from memory unless the file has changed since.
    pub fn doing(&self) -> Arc<Doing> {
        let Some(path) = self.cfg.doing_file.as_deref() else {
            return Arc::new(Doing::default());
        };
        let path = std::path::Path::new(path);
        let at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut held = self.timeline.lock();
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

    /// The corpus survey, from memory unless the file has changed. `None` is a real
    /// answer: "not mined yet" is not "an artefact saying zero".
    pub fn reading(&self) -> Option<Arc<CorpusRead>> {
        let path = self.cfg.reading_file.as_deref()?;
        let path = std::path::Path::new(path);
        let at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut held = self.reading.lock();
        if let Some(cached) = held.as_ref()
            && cached.at == at
        {
            return cached.reading.clone();
        }
        let reading = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<CorpusRead>(&text).ok())
            .map(Arc::new);
        *held = Some(CachedReading {
            at,
            reading: reading.clone(),
        });
        reading
    }

    pub fn create_oauth_state(&self, return_to: Option<String>) -> String {
        let mut bytes = [0u8; 24];
        rand::rng().fill_bytes(&mut bytes);
        let state = hex::encode(bytes);
        let mut map = self.oauth.lock();
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
        let mut map = self.oauth.lock();
        let entry = map.remove(state)?;
        if entry.created.elapsed() > OAUTH_TTL {
            return None;
        }
        Some(entry)
    }
}
