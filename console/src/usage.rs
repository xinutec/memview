//! How much of the subscription is spent, and how long until it comes back.
//!
//! Measured by the console itself: every `rate_limit_event` the CLI writes carries
//! a `utilization` straight off the API's response headers, so a console with
//! anything working knows the account's position within seconds. See
//! [`crate::protocol::Event::Limit`].
//!
//! The home dashboard is still read, and JUDGED against the live reading rather
//! than fallen back on: both go to [`fresher`], which asks of two readings of one
//! window which is later. The age and the host travel with whichever won, and a
//! window that has already reset reports no countdown.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::RwLock;

use crate::session::Seen;

/// How often to ask the dashboard: the reading behind it changes when somebody
/// opens a terminal.
const EVERY: Duration = Duration::from_secs(300);

/// Long enough that a sleeping dashboard cannot hold up the loop, short enough
/// that the loop is still periodic.
const PATIENCE: Duration = Duration::from_secs(10);

/// A reading exactly as the dashboard publishes it. Public so [`reading`]'s
/// arithmetic can be tested without a dashboard to ask.
#[derive(Debug, Clone, Deserialize)]
pub struct Published {
    pub host: String,
    /// RFC 3339, when the reading was taken.
    pub ts: String,
    pub five_hour_pct: f64,
    pub five_hour_resets_at: String,
    pub seven_day_pct: f64,
    pub seven_day_resets_at: String,
    /// The writer's own claim about provenance: a measurement (the API's figure at an
    /// instant the writer could date — home schema v9) or an echo of cached headers.
    /// Defaulted false, so a writer that does not say claims the weaker kind.
    #[serde(default)]
    pub measured: bool,
}

/// One rate-limit window, as the console shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Window {
    pub pct: f64,
    /// How long until this window turns over, in milliseconds. Absent once it has
    /// passed: the percentage belonged to a window that no longer exists, and since a
    /// reading arrives hours late as a matter of course, that is the ordinary case.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub resets_in_ms: Option<i64>,
}

/// One window that belongs to a single model rather than to the plan. Named by the
/// model, not a key: as of CLI 2.1.226 the scope arrives in a `model_scoped`
/// array carrying its own `display_name`, so the name is data. See
/// [`crate::protocol::usage_reply`].
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
    /// Which machine took it: the number is account-wide, and the machine is the only
    /// local part.
    pub host: String,
    /// How old the reading is, in milliseconds.
    pub age_ms: i64,
    /// Absent is a third state, not an expired window: an event names one window at a
    /// time, so the week can be known and the five hours unheard of — drawn as no row
    /// rather than a row saying something untrue.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub five_hour: Option<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub seven_day: Option<Window>,
    /// The windows belonging to one model, in name order so the strip does not
    /// reshuffle between polls. Empty for a dashboard reading — see [`merged`].
    pub models: Vec<Scoped>,
}

/// The dashboard's latest reading, kept here so a client never waits on it.
pub struct Usage {
    /// Absent when nothing said where to look; the front page then has no usage.
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

    /// What to show, given what the sessions have heard. See [`merged`]. Ages and
    /// countdowns are computed against THIS machine's clock rather than sent as
    /// instants: a phone's clock drifts, and "has this window turned over?" would
    /// then differ between screens.
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

    /// Keep asking, quietly. A dashboard asleep or unreachable is the normal state,
    /// so a failure is logged once at debug and the front page goes without.
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
/// First hand is not current: a session answers from its process's cached
/// headers, so an idle one truthfully reports a stale account, and reports it now.
/// Per window, since an event names one window.
///
/// Which of two readings of one window is current, in order:
///
/// - A LATER window instance wins outright.
/// - Within one instance the figure only RISES, so the higher reading is later.
/// - A FALL is believed only from a `measured` reading — a real request, or the
///   dashboard. A `get_usage` reply is a cache of unknowable age: it may raise the
///   figure, never lower it.
/// - A higher reading wins whatever its source, or the figure stops tracking your
///   own messages.
/// - An EQUAL reading still wins, on arrival time: `at` is when this was last
///   confirmed.
///
/// `resets_at` drifts between two readings of one instance, so it is compared
/// within [`SAME_WINDOW`]. The two instants wear different types
/// ([`crate::session::ResetsAt`], [`crate::session::Heard`]) so the arms cannot
/// be written the wrong way round.
pub fn fresher(held: &Seen, candidate: &Seen) -> bool {
    match (held.resets_at, candidate.resets_at) {
        // A different reset instant is a later window instance, and wins outright.
        (Some(theirs), Some(ours)) if !same_window(theirs, ours) => ours > theirs,
        // Rose: believed from any source — the arm a fresh `get_usage` wins on as you work.
        (Some(_), Some(_)) if candidate.utilization > held.utilization => true,
        // Fell within one window: only a measurement at least as recent may say so.
        // An echo falling is the flap.
        (Some(_), Some(_)) if candidate.utilization < held.utilization => {
            candidate.measured && candidate.at >= held.at
        }
        // Equal, or a reading with no reset time to place it: nothing to choose
        // but which arrived later.
        _ => candidate.at > held.at,
    }
}

/// How far two readings may disagree about when one window ends and still be
/// the same window. A minute: thirty times the drift seen, and a three hundredth
/// of the smallest real turnover (five hours). Too tight held a stale reading for
/// an hour (#814).
const SAME_WINDOW: i64 = 60;

/// Whether two reset instants describe one window instance. See [`SAME_WINDOW`].
fn same_window(held: crate::session::ResetsAt, candidate: crate::session::ResetsAt) -> bool {
    (held.0 - candidate.0).abs() <= SAME_WINDOW
}

/// Fold what the sessions have just said into what is already known.
///
/// A reading outlives the session that heard it: gathered fresh per poll, the
/// figure showed 92 → 93 → 92 as a session ended (memview #87). Safe because
/// [`fresher`] decides each window — the figure still falls exactly when it should.
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

/// The dashboard's word about one window, in the same terms as a live reading, so
/// [`fresher`] can judge the two by one rule.
fn published_as_seen(pct: f64, resets_at: &str, ts: &str, measured: bool) -> Option<Seen> {
    Some(Seen {
        // A percentage there, a fraction here.
        utilization: pct / 100.0,
        resets_at: Some(crate::session::ResetsAt(at(resets_at)? / 1000)),
        at: crate::session::Heard(at(ts)?),
        // The row's own claim, not this console's assertion: a row that does not say
        // (pre home v9) is an echo, which may fill in but never lower. The date is ITS
        // host's capture instant, so the row this console published about itself comes
        // back with the stamp it went out with and cannot displace anything.
        measured,
    })
}

pub fn merged(
    seen: &BTreeMap<String, Seen>,
    dashboard: Option<&Published>,
    now_ms: i64,
) -> Option<Reading> {
    let published = dashboard.map(|it| reading(it, now_ms));
    // Judged, not fallen back on: `live.or_else(published)` preferred an hour-old
    // own reading over a six-minute-old published one. Each reading carries the
    // machine that took it, because the winner decides what the age and host lines
    // are about.
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
    // The console's own hearing only: the dashboard's copy of a model's window comes
    // FROM here, so there is nothing to judge it against.
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
    // Nothing known about either plan window is nothing to show; a lone scoped bar
    // over no context is not a reading anybody could act on.
    five_hour.as_ref().or(seven_day.as_ref())?;
    // The age and host of what is ON SCREEN: the newer of the two chosen readings.
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

/// What marks a window as one model's. The windows share one map keyed by the
/// CLI's names, so a model's `display_name` is prefixed rather than dropped in
/// beside them where `five_hour` would collide. A colon: no CLI window name has one.
const MODEL_PREFIX: &str = "model:";

/// How a model's window is filed among the rest.
pub fn model_key(display_name: &str) -> String {
    format!("{MODEL_PREFIX}{display_name}")
}

/// The machine this console runs on, for a reading it took itself. A real name
/// rather than "this console": home's `claude_usage` table is keyed by host and
/// serves the freshest row ACROSS hosts, so a constant would win every comparison
/// under a name no machine answers to. Resolved once.
static HERE: LazyLock<String> = LazyLock::new(|| short_name(&hostname()));

/// What this machine calls itself, verbatim, or empty if the kernel will not say.
/// POSIX lets `gethostname` fill the buffer without a terminator and return 0,
/// so the buffer is over-sized and the last byte forced to nul.
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

/// The name the rest of the fleet uses: the short form, since `network.nix`, the
/// `.vpn` names and home's rows all use it, and the long form would open a second
/// row for a host that already has one. Split from the syscall so a test can pin it.
pub fn short_name(raw: &str) -> String {
    let short = raw.split('.').next().unwrap_or_default();
    if short.is_empty() {
        return UNKNOWN_HOST.to_string();
    }
    short.to_string()
}

/// What this console runs on, as it will be attributed. Public so a test need not
/// hard-code the machine it happens to run on.
pub fn here() -> &'static str {
    &HERE
}

/// Never silently empty: a blank provenance reads as "no machine".
const UNKNOWN_HOST: &str = "unknown host";

fn live(seen: Option<&Seen>, now_ms: i64) -> Option<Window> {
    let seen = seen?;
    Some(Window {
        // A fraction on the wire, a percentage on a screen — the same conversion
        // the CLI does on its way to a status line.
        pct: seen.utilization * 100.0,
        // Seconds there, milliseconds here: the conversion lives on the type, because a
        // CLI epoch second read as a millisecond puts every reset in 1970.
        resets_in_ms: seen
            .resets_at
            .map(|turns| turns.in_ms() - now_ms)
            .filter(|left| *left > 0),
    })
}

/// What a published reading says, as of `now_ms`. The pure half: ages and
/// countdowns worked out here, in milliseconds so a test states its now as a number.
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
        // Never read back from the dashboard: home's per-model figures are this
        // console's own, pushed there by `claude_usage_push.py`.
        models: Vec::new(),
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

/// How sessions are ranked when one must be asked what the account has spent.
/// Highest wins.
///
/// Idleness first, recency second: the most recent speaker holds the freshest
/// cache but is very nearly the one working now, and a busy CLI answers no control
/// request until its turn ends (asked 2.0 s into a turn, answered at 8.5 s —
/// memview #817). A session that has just finished answers at once.
pub fn asked_before(working: bool, last_heard: i64) -> (bool, i64) {
    (!working, last_heard)
}
