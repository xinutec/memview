//! Shared application state + the short-lived OAuth `state` store (in-memory,
//! per process — fine for a single-pod deployment). Copied from `messages`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rand::RngCore;

use crate::config::Config;
use crate::share::ShareStore;
use reader::doing::Doing;
use reader::effects::Effects;
use reader::reading::CorpusRead;

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

/// How many effects each `(agent, minute)` has, keyed by the EFFECTS artefact's
/// own agent index — the timeline's per-row count.
///
/// Named because it travels with the artefact it is derived from and the pair
/// is returned together; spelled out at both ends it was the kind of type
/// signature nobody reads.
pub type EffectCounts = Arc<HashMap<(u32, i64), u32>>;

/// The same, for the effects — and the argument is stronger, not weaker: this
/// artefact is 35 MB where the timeline is 10, and it is opened by exactly the
/// gesture that is meant to feel instant, tapping a row to see what it did.
struct CachedEffects {
    at: Option<std::time::SystemTime>,
    effects: Arc<Effects>,
    /// How many effects each `(agent, minute)` has — the timeline's per-row
    /// count, built once with the artefact rather than per request.
    ///
    /// ⚠ **Built here because the alternative is a full pass per request.**
    /// A page is 200 moments and the artefact is 327,852 rows, so counting them
    /// where the page is assembled means walking the whole thing to answer 200
    /// questions, on the request that is meant to feel instant. 47,033 keys.
    counts: EffectCounts,
}

/// The corpus survey, held the same way and for a weaker reason than the other
/// two: it is 7 kB, so re-reading it per request would cost almost nothing.
///
/// Cached anyway because the mtime check is the part that matters — it is what
/// makes "the artefact changed" the only way a served number can change, so a
/// figure on a page and a figure in `--bin shell-files` can only disagree by
/// being from different nights, never by one of them having recomputed.
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

    /// The effects, from memory unless the file has changed since.
    ///
    /// Absent config means an empty artefact rather than an error: a deployment
    /// that has not mined one yet should serve a timeline with nothing under it,
    /// not a 500 on a page that otherwise works.
    pub fn effects(&self) -> Arc<Effects> {
        self.effects_and_counts().0
    }

    /// The effects and the per-`(agent, minute)` count, from one cache.
    ///
    /// Together rather than apart: they are derived from the same file and go
    /// stale at the same instant, and two caches keyed off the same mtime is a
    /// second chance to serve counts that describe an artefact no longer held.
    pub fn effects_and_counts(&self) -> (Arc<Effects>, EffectCounts) {
        let Some(path) = self.cfg.effects_file.as_deref() else {
            return (Arc::new(Effects::default()), Arc::new(HashMap::new()));
        };
        let path = std::path::Path::new(path);
        let at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut held = self.effects.lock().expect("effects poisoned");
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

    /// The corpus survey, from memory unless the file has changed since.
    ///
    /// ⚠ **`None` is a real answer and is not an error.** A deployment that has
    /// never mined one — a fresh checkout, or isis before the first sync — must
    /// say "not mined yet" on the page rather than 500 it. The distinction the
    /// view needs is between "no artefact" and "an artefact saying zero", and a
    /// default-filled `CorpusRead` would erase exactly that.
    pub fn reading(&self) -> Option<Arc<CorpusRead>> {
        let path = self.cfg.reading_file.as_deref()?;
        let path = std::path::Path::new(path);
        let at = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut held = self.reading.lock().expect("reading poisoned");
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
