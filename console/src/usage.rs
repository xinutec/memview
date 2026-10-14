//! How much of the subscription is spent, and how long until it comes back.
//!
//! Heard first-hand from the sessions' rate-limit readings, and judged against the
//! home dashboard's by [`fresher`]; the winner's age and host travel with it.

#![expect(unsafe_code, reason = "libc::gethostname into a fixed buffer")]

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::RwLock;

use crate::session::{Heard, Seen};

/// How often to ask the dashboard.
const EVERY: Duration = Duration::from_secs(300);

/// How long a sleeping dashboard may hold up the loop.
const PATIENCE: Duration = Duration::from_secs(10);

/// A reading exactly as the dashboard publishes it.
#[derive(Debug, Clone, Deserialize)]
pub struct Published {
    pub host: String,
    /// RFC 3339, when the reading was taken.
    pub ts: String,
    pub five_hour_pct: f64,
    pub five_hour_resets_at: String,
    pub seven_day_pct: f64,
    pub seven_day_resets_at: String,
    /// Whether the writer claims a measurement; unsaid is an echo.
    #[serde(default)]
    pub measured: bool,
}

/// One rate-limit window, as the console shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Window {
    pub pct: f64,
    /// How long until this window turns over, in milliseconds; absent once passed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub resets_in_ms: Option<i64>,
}

/// One window belonging to a single model, named by the CLI.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Scoped {
    /// The model's own display name, as the CLI gives it — "Fable".
    pub model: String,
    #[serde(flatten)]
    pub window: Window,
}

/// What the client is told.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Reading {
    /// Which machine took the reading.
    pub host: String,
    /// How old the reading is, in milliseconds.
    pub age_ms: i64,
    /// Absent when this window has not been heard of — no row, not a false one.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub five_hour: Option<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub seven_day: Option<Window>,
    /// The per-model windows, in name order so the strip holds still.
    pub models: Vec<Scoped>,
}

/// The dashboard's latest reading, kept here so a client never waits on it.
pub struct Usage {
    /// Absent when not configured.
    url: Option<String>,
    latest: RwLock<Option<Published>>,
}

impl Usage {
    pub fn new(url: Option<String>) -> Self {
        Self {
            url,
            latest: RwLock::new(None),
        }
    }

    /// What to show, given what the sessions heard — [`merged`]. Ages and countdowns
    /// use this machine's clock, not a drifting phone's.
    pub async fn reading(&self, seen: &BTreeMap<String, Seen>) -> Option<Reading> {
        merged(seen, self.latest.read().await.as_ref(), now_ms())
    }

    /// Ask the dashboard once.
    async fn fetch(&self, client: &reqwest::Client) -> anyhow::Result<()> {
        let Some(url) = &self.url else {
            return Ok(());
        };
        let published: Published = client.get(url).send().await?.json().await?;
        *self.latest.write().await = Some(published);
        Ok(())
    }

    /// Keep asking, quietly: an unreachable dashboard is normal.
    pub fn watch(self: Arc<Self>) {
        if self.url.is_none() {
            return;
        }
        let Ok(client) = reqwest::Client::builder().timeout(PATIENCE).build() else {
            tracing::warn!("no HTTP client, so no usage will be shown");
            return;
        };
        tokio::spawn(async move {
            loop {
                if let Err(failure) = self.fetch(&client).await {
                    tracing::debug!("usage unavailable: {failure}");
                }
                tokio::time::sleep(EVERY).await;
            }
        });
    }
}

/// Which of two readings of one window is current — first-hand is not current: an
/// idle session repeats cached headers. In order:
///
/// - A later window instance wins outright.
/// - Within one instance the figure only rises, so the higher reading is later.
/// - A fall is believed only from a `measured` reading — a real request, or the
///   dashboard. A `get_usage` reply is a cache of unknowable age: it may raise the
///   figure, never lower it.
/// - A higher reading wins whatever its source, or the figure stops tracking your
///   own messages.
/// - An equal reading still wins, on arrival time: `at` is when this was last
///   confirmed.
///
/// `resets_at` drifts, so it is compared within [`SAME_WINDOW`].
pub fn fresher(held: &Seen, candidate: &Seen) -> bool {
    match (held.resets_at, candidate.resets_at) {
        // A different reset instant is a later window instance, and wins outright.
        (Some(theirs), Some(ours)) if !same_window(theirs, ours) => ours > theirs,
        // Rose: believed from any source.
        (Some(_), Some(_)) if candidate.utilization > held.utilization => true,
        // Fell: only a measurement at least as recent; a falling echo is stale.
        (Some(_), Some(_)) if candidate.utilization < held.utilization => {
            candidate.measured && candidate.at >= held.at
        }
        // Equal, or unplaceable: the later arrival.
        _ => candidate.at > held.at,
    }
}

/// How far two readings' reset times may differ and still be one window.
const SAME_WINDOW: i64 = 60;

/// Whether two reset instants describe one window instance. See [`SAME_WINDOW`].
fn same_window(held: crate::session::ResetsAt, candidate: crate::session::ResetsAt) -> bool {
    (held.0 - candidate.0).abs() <= SAME_WINDOW
}

/// When a `get_usage` answer was true, and whether it is a measurement: fetched
/// means now; otherwise as old as the API's last answer to that process, if any.
pub fn echo_dated(fetched: bool, answered: Option<i64>, now: i64) -> (Heard, bool) {
    match (fetched, answered) {
        (true, _) => (Heard(now), true),
        (false, Some(at)) => (Heard(at), true),
        (false, None) => (Heard(now), false),
    }
}

/// Fold what the sessions just said into what is known; a reading outlives the
/// session that heard it.
pub fn remember(
    known: &mut BTreeMap<String, Seen>,
    heard: impl IntoIterator<Item = (String, Seen)>,
) {
    for (window, seen) in heard {
        match known.get(&window) {
            Some(held) if !fresher(held, &seen) => {}
            _ => {
                known.insert(window, seen);
            }
        }
    }
}

/// The dashboard's word about one window, in a live reading's terms.
fn published_as_seen(pct: f64, resets_at: &str, ts: &str, measured: bool) -> Option<Seen> {
    Some(Seen {
        // A percentage there, a fraction here.
        utilization: pct / 100.0,
        resets_at: Some(crate::session::ResetsAt(at(resets_at)? / 1000)),
        at: crate::session::Heard(at(ts)?),
        // Dated by its host, so this console's own row displaces nothing.
        measured,
    })
}

pub fn merged(
    seen: &BTreeMap<String, Seen>,
    dashboard: Option<&Published>,
    now_ms: i64,
) -> Option<Reading> {
    let published = dashboard.map(|it| reading(it, now_ms));
    // Judged, not fallen back on: an old own reading must not beat a newer one.
    let pick = |mine: Option<&Seen>, theirs: Option<Seen>| -> Option<(Seen, String)> {
        let theirs = theirs.map(|it| {
            (
                it,
                dashboard.map_or_else(|| HERE.clone(), |d| d.host.clone()),
            )
        });
        match (mine, theirs) {
            (Some(mine), Some(theirs)) if fresher(mine, &theirs.0) => Some(theirs),
            (Some(mine), _) => Some((mine.clone(), HERE.clone())),
            (None, theirs) => theirs,
        }
    };
    let of = |pct: fn(&Published) -> f64, resets: fn(&Published) -> &str| {
        dashboard.and_then(|it| published_as_seen(pct(it), resets(it), &it.ts, it.measured))
    };
    let chosen_five = pick(
        seen.get(FIVE_HOUR),
        of(|d| d.five_hour_pct, |d| &d.five_hour_resets_at),
    );
    let chosen_seven = pick(
        seen.get(SEVEN_DAY),
        of(|d| d.seven_day_pct, |d| &d.seven_day_resets_at),
    );
    let five_hour = live(chosen_five.as_ref().map(|it| &it.0), now_ms);
    let seven_day = live(chosen_seven.as_ref().map(|it| &it.0), now_ms);
    // Own hearing only: the dashboard's copy came from here.
    let models: Vec<Scoped> = seen
        .iter()
        .filter_map(|(window, heard)| Some((window.strip_prefix(MODEL_PREFIX)?, heard)))
        .filter_map(|(model, heard)| {
            Some(Scoped {
                model: model.to_string(),
                window: live(Some(heard), now_ms)?,
            })
        })
        .collect();
    // Neither plan window known: nothing to show.
    five_hour.as_ref().or(seven_day.as_ref())?;
    // Age and host: the newer of the two chosen readings.
    let newest = [chosen_five.as_ref(), chosen_seven.as_ref()]
        .into_iter()
        .flatten()
        .max_by_key(|it| it.0.at);
    Some(match newest {
        Some((seen, host)) => Reading {
            host: host.clone(),
            age_ms: (now_ms - seen.at.0).max(0),
            five_hour,
            seven_day,
            models,
        },
        None => Reading {
            five_hour,
            seven_day,
            models,
            ..published?
        },
    })
}

/// The CLI's own names for the two windows worth showing.
const FIVE_HOUR: &str = "five_hour";
const SEVEN_DAY: &str = "seven_day";

/// The prefix filing a model's window apart from the plan's; no CLI name has a colon.
const MODEL_PREFIX: &str = "model:";

/// How a model's window is filed among the rest.
pub fn model_key(display_name: &str) -> String {
    format!("{MODEL_PREFIX}{display_name}")
}

/// The machine this console runs on — a real host name, as home's table keys by host.
static HERE: LazyLock<String> = LazyLock::new(|| short_name(&hostname()));

/// What this machine calls itself, or empty. The buffer's last byte is forced to
/// nul: `gethostname` may not terminate.
fn hostname() -> String {
    const LEN: usize = 256;
    let mut buf = [0_i8; LEN];
    // SAFETY: `buf` is `LEN` bytes and the length passed matches it.
    if unsafe { libc::gethostname(buf.as_mut_ptr(), LEN) } != 0 {
        return String::new();
    }
    buf[LEN - 1] = 0;
    // SAFETY: nul-terminated above, whatever the kernel wrote.
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

/// The short host name the rest of the fleet uses.
pub fn short_name(raw: &str) -> String {
    let short = raw.split('.').next().unwrap_or_default();
    if short.is_empty() {
        return UNKNOWN_HOST.to_string();
    }
    short.to_string()
}

/// What this console runs on, as it will be attributed.
pub fn here() -> &'static str {
    &HERE
}

/// Never silently empty: a blank provenance reads as "no machine".
const UNKNOWN_HOST: &str = "unknown host";

fn live(seen: Option<&Seen>, now_ms: i64) -> Option<Window> {
    let seen = seen?;
    Some(Window {
        // A fraction on the wire, a percentage on screen.
        pct: seen.utilization * 100.0,
        // Seconds there, milliseconds here.
        resets_in_ms: seen
            .resets_at
            .map(|turns| turns.in_ms() - now_ms)
            .filter(|left| *left > 0),
    })
}

/// What a published reading says, as of `now_ms`.
pub fn reading(published: &Published, now_ms: i64) -> Reading {
    Reading {
        host: published.host.clone(),
        // Unreadable means no age rather than 1970.
        age_ms: at(&published.ts).map(|then| now_ms - then).unwrap_or(0),
        five_hour: Some(window(
            published.five_hour_pct,
            &published.five_hour_resets_at,
            now_ms,
        )),
        seven_day: Some(window(
            published.seven_day_pct,
            &published.seven_day_resets_at,
            now_ms,
        )),
        // Never read back: home's per-model figures came from here.
        models: Vec::new(),
    }
}

fn window(pct: f64, resets_at: &str, now_ms: i64) -> Window {
    Window {
        pct,
        // Already turned over: no window.
        resets_in_ms: at(resets_at)
            .map(|turns| turns - now_ms)
            .filter(|left| *left > 0),
    }
}

/// An RFC 3339 stamp as milliseconds since the epoch.
fn at(stamp: &str) -> Option<i64> {
    let when = OffsetDateTime::parse(stamp, &Rfc3339).ok()?;
    Some((when.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// Now, by this machine's clock.
fn now_ms() -> i64 {
    (OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
}

/// How sessions rank to be asked what the account has spent, highest first: idle,
/// then recent — a busy CLI answers only after its turn.
pub fn asked_before(working: bool, last_heard: i64) -> (bool, i64) {
    (!working, last_heard)
}
