//! How much of the subscription is spent, and how long until it comes back.
//!
//! **The console measures this itself, from the sessions it is already running.**
//! Every `rate_limit_event` the CLI writes to stdout carries a `utilization`
//! taken straight off the API's response headers
//! (`anthropic-ratelimit-unified-5h-utilization` and its siblings), so a console
//! with anything working knows the account's position within seconds of the last
//! request. See [`crate::protocol::Event::Limit`].
//!
//! ⚠ **This module first shipped believing the opposite**, on the strength of a
//! comment in `protocol.rs` saying the percentages existed only in the
//! statusLine hook's input. They do not: the hook and the stream are fed from
//! the same headers. The console had been receiving the number all along and
//! discarding it for want of a field to put it in, while reading a copy off the
//! home dashboard that was routinely five hours stale — because a status line
//! belongs to a terminal and these sessions are headless.
//!
//! The dashboard is still read, and still useful, but only as the **fallback for
//! a window nothing has reported yet**: a console just started, or one whose
//! sessions have all been idle since a window last turned over. A reading from
//! it can be hours old, so the age travels with the number either way, and a
//! window that has already reset reports no figure at all rather than the one it
//! held before it turned over.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::RwLock;

use crate::session::Seen;

/// How often to ask the dashboard. The reading behind it changes when somebody
/// opens a terminal, so anything faster is asking a question whose answer is
/// already known.
const EVERY: Duration = Duration::from_secs(300);

/// Long enough that a sleeping dashboard cannot hold up the loop, short enough
/// that the loop is still periodic.
const PATIENCE: Duration = Duration::from_secs(10);

/// A reading exactly as the dashboard publishes it.
///
/// Public because [`reading`] is: the arithmetic that turns instants into
/// durations is the part worth testing, and it should be testable without a
/// dashboard to ask.
#[derive(Debug, Clone, Deserialize)]
pub struct Published {
    pub host: String,
    /// RFC 3339, when the reading was taken.
    pub ts: String,
    pub five_hour_pct: f64,
    pub five_hour_resets_at: String,
    pub seven_day_pct: f64,
    pub seven_day_resets_at: String,
}

/// One rate-limit window, as the console shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    pub pct: f64,
    /// How long until this window turns over, in milliseconds.
    ///
    /// ⚠ **Absent once it has passed, and that is not a formatting detail.** A
    /// percentage belongs to a window; when the window resets the percentage
    /// goes back to nothing, and a reading taken before the turn describes a
    /// window that no longer exists. Since this arrives hours late as a matter
    /// of course, that is the ordinary case rather than an edge one — so the
    /// figure is withheld here rather than drawn as though it still meant
    /// something.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resets_in_ms: Option<i64>,
}

/// What the client is told.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Reading {
    /// Which machine took it. Shown because the number is account-wide and the
    /// machine is the only part of it that is local.
    pub host: String,
    /// How old the reading is, in milliseconds.
    pub age_ms: i64,
    /// ⚠ **Absent is a third state, and not the same as an expired window.** An
    /// event names one window at a time, so a console can know the week's figure
    /// and have heard nothing at all about the five hours — which is "no reading"
    /// rather than "reset since", and is drawn as no row rather than as a row
    /// saying something untrue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<Window>,
}

/// The dashboard's latest reading, kept here so a client never waits on it.
pub struct Usage {
    /// Absent when nothing told this console where to look, in which case the
    /// front page simply has no usage on it.
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

    /// What to show, given what the sessions have heard. See [`merged`].
    ///
    /// ⚠ **Ages and countdowns are computed here, against THIS machine's
    /// clock**, rather than sent as instants for the client to subtract from its
    /// own. A phone's clock drifts, and the one case that matters — "has this
    /// window already turned over?" — would then be answered differently on
    /// different screens.
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

    /// Keep asking, quietly.
    ///
    /// A dashboard that is asleep, unreachable over the VPN, or has never been
    /// told anything is the normal state of this — so a failure is logged once
    /// at debug and the front page goes without. Nothing here is load-bearing:
    /// the console's own work does not depend on a number it did not measure.
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

/// What the console itself has heard, with the dashboard behind it.
///
/// ⚠ **The live half is the one that matters, and it was there all along.** Each
/// `rate_limit_event` on a session's stdout carries a `utilization` straight off
/// the API's own response headers, so the console measures this at first hand
/// for every request its sessions make. The dashboard is kept only for a window
/// nothing has reported yet — a console just started, or one whose sessions have
/// all been idle since the weekly window last turned over.
///
/// Per window rather than whole: an event names one window (the *representative*
/// one), so the five-hour figure can be seconds old while the weekly one is
/// still the dashboard's.
/// Which of two readings of the same window is the current one.
///
/// ⚠ **Not the one that arrived last, and that is the whole point.** Every
/// session answers `get_usage` from its own process's cached rate-limit headers,
/// which are as old as that process's last request to the API — so an idle
/// session truthfully reports the account as it stood an hour ago, and it
/// reports it *now*. Taking the newest arrival made a stale answer authoritative
/// for a minute at a time: measured on the phone as the week's figure flipping
/// 81 → 77 → 81 → 77 on the console's sixty-second beat, with one sample showing
/// the two windows disagreeing about which hour it was.
///
/// What breaks the tie is the figure itself. Utilisation only rises inside one
/// window, so of two readings of the *same* window instance the higher one is
/// the later one, whoever heard it. A window that has turned over is a different
/// instance with a later `resets_at`, and there the newer instance wins outright
/// — otherwise the old window's high-water mark would outrank the fresh window's
/// honest 3%.
///
/// Arrival time is the fallback for a reading with no reset time at all, which
/// is what a `rate_limit_event` carries when the CLI declines to say.
pub fn fresher(held: &Seen, candidate: &Seen) -> bool {
    match (held.resets_at, candidate.resets_at) {
        (Some(theirs), Some(ours)) if ours != theirs => ours > theirs,
        (Some(_), Some(_)) => candidate.utilization > held.utilization,
        _ => candidate.at > held.at,
    }
}

pub fn merged(
    seen: &BTreeMap<String, Seen>,
    dashboard: Option<&Published>,
    now_ms: i64,
) -> Option<Reading> {
    let published = dashboard.map(|it| reading(it, now_ms));
    let five_hour = live(seen.get(FIVE_HOUR), now_ms)
        .or_else(|| published.as_ref().and_then(|it| it.five_hour.clone()));
    let seven_day = live(seen.get(SEVEN_DAY), now_ms)
        .or_else(|| published.as_ref().and_then(|it| it.seven_day.clone()));
    // Nothing known about either window is nothing to show. One is worth showing.
    five_hour.as_ref().or(seven_day.as_ref())?;
    // The freshest thing on screen is what the age line is about, and when the
    // console has heard anything at all that is the console.
    let heard = [seen.get(FIVE_HOUR), seen.get(SEVEN_DAY)]
        .into_iter()
        .flatten()
        .map(|it| it.at)
        .max();
    Some(match heard {
        Some(at) => Reading {
            host: HERE.to_string(),
            age_ms: (now_ms - at).max(0),
            five_hour,
            seven_day,
        },
        None => Reading {
            five_hour,
            seven_day,
            ..published?
        },
    })
}

/// The CLI's own names for the two windows worth showing.
const FIVE_HOUR: &str = "five_hour";
const SEVEN_DAY: &str = "seven_day";

/// What to call a reading this console took itself. Not a hostname: the figure
/// is account-wide, so the only useful provenance is *how* it was come by.
const HERE: &str = "this console";

fn live(seen: Option<&Seen>, now_ms: i64) -> Option<Window> {
    let seen = seen?;
    Some(Window {
        // A fraction on the wire, a percentage on a screen — the same conversion
        // the CLI does on its way to a status line.
        pct: seen.utilization * 100.0,
        // Seconds here, milliseconds everywhere else in this file: the CLI's
        // `resetsAt` is an epoch second, and treating it as milliseconds puts
        // every reset in 1970 and reports every window as already turned over.
        resets_in_ms: seen
            .resets_at
            .map(|turns| turns * 1000 - now_ms)
            .filter(|left| *left > 0),
    })
}

/// What a published reading says, as of `now_ms`.
///
/// The pure half, and the whole of the arithmetic: ages and countdowns are
/// worked out here so that a client is handed durations rather than instants to
/// subtract from a clock of its own.
///
/// Milliseconds rather than a date type, so a test states its "now" as a number
/// and reads the answer as one.
pub fn reading(published: &Published, now_ms: i64) -> Reading {
    Reading {
        host: published.host.clone(),
        // Unreadable means no age rather than 1970: a stamp we cannot parse is
        // an unknown, and the epoch is a very confident wrong answer.
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
    }
}

fn window(pct: f64, resets_at: &str, now_ms: i64) -> Window {
    Window {
        pct,
        // Not in the future means it has already turned over, which is no
        // window at all.
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
