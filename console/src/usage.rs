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
//! The dashboard is still read, and it is **judged against** the live reading
//! rather than kept behind it. It was a fallback for a window nothing had
//! reported yet — a console just started, or one whose sessions have all been
//! idle since a window turned over — and that made *absent* the only condition
//! under which it was consulted, so an hour-old live figure outranked a
//! six-minute-old published one (#113). Both are now put to [`fresher`], which
//! is the same question either way: of two readings of one window, which is the
//! later. A reading from either side can be hours old, so the age and the host
//! travel with whichever won, and a window that has already reset reports no
//! countdown rather than one that has run out.

use std::collections::BTreeMap;
use std::ffi::CStr;
use std::sync::{Arc, LazyLock};
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

/// One window that belongs to a single model rather than to the plan.
///
/// ⚠ **Named by the model, not by a key.** The CLI used to file these under
/// fixed window names (`seven_day_opus`, `seven_day_sonnet`); as of 2.1.226 those
/// are null and the live scope arrives in a `model_scoped` array carrying its own
/// `display_name`. So the name is data — it is shown verbatim, and nothing here
/// or downstream knows which models exist. See [`crate::protocol::usage_reply`].
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Scoped {
    /// The model's own display name, as the CLI gives it — "Fable".
    pub model: String,
    #[serde(flatten)]
    pub window: Window,
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
    /// The windows belonging to one model, in name order so the strip does not
    /// reshuffle between polls. Empty for a reading that came from the dashboard,
    /// which carries none — see [`merged`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<Scoped>,
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
/// for every request its sessions make — but *first hand* is not the same as
/// *current*, which is the mistake this used to make. A session answers from its
/// own process's cached headers, so a console with nothing but idle sessions
/// measures the account as it stood whenever one of them last spoke to the API.
/// The dashboard is therefore compared with, not fallen back on.
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
///
/// The two instants wear different types ([`crate::session::ResetsAt`] and
/// [`crate::session::Heard`]) so that the arms below cannot be written the wrong
/// way round: the mistake this function exists to correct no longer compiles.
/// ⚠ **A reading that merely CONFIRMS the one held still wins, on its arrival
/// time.** `candidate.utilization > held.utilization` alone discarded an equal
/// figure — and with it the fact that somebody had just heard it — so `at`
/// stopped being "when this was last true" and became "when it last went up".
/// Everything downstream reads that as the age of the reading, so a figure
/// reconfirmed a minute ago was drawn as an hour old, teaching you to distrust a
/// number that was fine. The two questions are separate and are answered
/// separately: *which reading is truest* is the higher utilisation, *how long ago
/// was it confirmed* is the most recent arrival that said so.
///
/// ⚠ **Two readings of one window do not agree to the second about when it
/// ends**, and equality was the test for "same instance" until #814. Measured
/// 2026-08-12: the console's sessions were being told `23:20:01` while two fresh
/// `get_usage` probes taken the same minute were told `23:19:59.838278` — and a
/// third, ten minutes earlier, `23:19:59.955616`. The instant drifts, and not in
/// one direction. Held to equality, whichever cohort reported the latest instant
/// latched the window shut: every reading from the other looked like an *older*
/// instance and was dropped whole, figure and arrival time together, until the
/// window really turned over hours later. See [`SAME_WINDOW`].
pub fn fresher(held: &Seen, candidate: &Seen) -> bool {
    match (held.resets_at, candidate.resets_at) {
        (Some(theirs), Some(ours)) if !same_window(theirs, ours) => ours > theirs,
        (Some(_), Some(_)) if candidate.utilization > held.utilization => true,
        (Some(_), Some(_)) if candidate.utilization < held.utilization => false,
        _ => candidate.at > held.at,
    }
}

/// How far two readings may disagree about when one window ends and still be
/// talking about the same window.
///
/// Generous on purpose, because the two errors are nothing like each other in
/// size. Being too tight is what #814 was: readings a second apart judged
/// different instances, and the reading held could not be displaced for an hour.
/// Being too loose would take a genuine turnover for a wobble — and the shortest
/// window there is runs five hours, so a turnover moves this instant by 18,000
/// seconds. A minute is thirty times the drift that has been seen and a three
/// hundredth of the smallest real step.
const SAME_WINDOW: i64 = 60;

/// Whether two reset instants describe one window instance. See [`SAME_WINDOW`].
fn same_window(held: crate::session::ResetsAt, candidate: crate::session::ResetsAt) -> bool {
    (held.0 - candidate.0).abs() <= SAME_WINDOW
}

/// Fold what the sessions have just said into what is already known.
///
/// ⚠ **A reading has to outlive the session that heard it.** The roster used to
/// gather this fresh from its live sessions on every poll, so a reading lasted
/// exactly as long as its source: when the session holding 93% ended, the
/// highest remaining was 92%, and the front page — polling every five seconds —
/// showed 92 → 93 → 92 while nothing about the account had changed (memview
/// #87). A figure that only ever moves one way is worth more than one that is
/// instantaneously right.
///
/// Safe to keep because [`fresher`] decides each window: utilisation only rises
/// inside an instance, so remembering the highest cannot go stale within one,
/// and a turned-over window is a later instance that displaces it outright. The
/// figure still falls exactly when it should.
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

/// The dashboard's word about one window, in the same terms as a live reading.
///
/// So that [`fresher`] can judge the two against each other. It is the same
/// question — which of two readings of this window is the current one — and
/// asking it in two places with two rules is how the console came to prefer an
/// hour-old figure it had measured over a six-minute-old one it had been told.
fn published_as_seen(pct: f64, resets_at: &str, ts: &str) -> Option<Seen> {
    Some(Seen {
        // A percentage there, a fraction here: `live` multiplies back up, and a
        // comparison between the two only means anything in one of them.
        utilization: pct / 100.0,
        resets_at: Some(crate::session::ResetsAt(at(resets_at)? / 1000)),
        at: crate::session::Heard(at(ts)?),
    })
}

pub fn merged(
    seen: &BTreeMap<String, Seen>,
    dashboard: Option<&Published>,
    now_ms: i64,
) -> Option<Reading> {
    let published = dashboard.map(|it| reading(it, now_ms));
    // ⚠ **The dashboard is judged, not merely fallen back on.** This read
    // `live(…).or_else(|| published…)`, which reaches for the published figure
    // only when the live one is ABSENT — and absent is not the same as older.
    // Measured 2026-08-07 19:57Z: the console drew 5h 13% / 7d 12% at an age of
    // 55 minutes while the dashboard, six minutes old and describing the same
    // window instance, said 22% and 14%. Nine points low, and preferring the
    // worse number because it happened to be its own.
    //
    // `fresher` already knows how to answer this — the higher figure inside one
    // window instance, the later instance across two — so it is asked here
    // rather than a second rule being written beside it.
    //
    // Each reading carries the machine that took it, because the winner decides
    // what the age line and the host line are about — a dashboard figure shown
    // under this console's name and this console's age would be the same lie
    // pointing the other way.
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
        dashboard.and_then(|it| published_as_seen(pct(it), resets(it), &it.ts))
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
    // ⚠ **The console's own hearing only, with nothing to judge it against.** A
    // model's window arrives in the same `get_usage` reply as the two above, so
    // it is exactly as fresh as they are — but the dashboard has no copy of it to
    // compare with, because the dashboard's copy comes FROM here. Putting it
    // through `pick` would be asking which of one reading is the later.
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
    // Nothing known about either window is nothing to show. One is worth showing.
    //
    // Judged on the two plan-wide windows alone, and a model's window cannot
    // rescue it: the three come back in one reply, so a console that has heard a
    // model scope has heard those too, and a lone scoped bar over no context is
    // not a reading anybody could act on.
    five_hour.as_ref().or(seven_day.as_ref())?;
    // ⚠ **The age and the host of what is ON SCREEN**, which is no longer always
    // this console's own. Taken from the newer of the two chosen readings, which
    // is the one a reader's eye goes to.
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

/// What marks a window as one model's rather than the plan's.
///
/// The windows share one map keyed by the CLI's own names, and a model's
/// `display_name` is not one of those — so it is prefixed rather than dropped in
/// beside them, where a model called `five_hour` would displace the plan's own
/// window. A colon because no CLI window name contains one.
const MODEL_PREFIX: &str = "model:";

/// How a model's window is filed among the rest.
pub fn model_key(display_name: &str) -> String {
    format!("{MODEL_PREFIX}{display_name}")
}

/// The machine this console runs on, for a reading it took itself.
///
/// ⚠ **This was the literal string "this console" until 2026-08-05**, on the
/// argument that the figure is account-wide so the only useful provenance is
/// *how* it was come by rather than *where*. The "how" is real and still wanted
/// — but `age_ms` already carries it, and better: a reading forty seconds old is
/// first-hand on its face, while a name never says how stale it is.
///
/// What forced a real name is that the reading is no longer only drawn. home's
/// `claude_usage` table is `PRIMARY KEY (host)` upserted, and its `GET
/// /api/usage` serves the freshest row *across* hosts — so a constant here
/// becomes a row that wins every freshness comparison under a name no machine
/// answers to, permanently stranding the row belonging to the host that really
/// took the reading.
///
/// Resolved once: the name cannot change while the process runs, and a console
/// that has to ask the kernel for it on the way to answering every request is
/// asking a question it already knows the answer to.
static HERE: LazyLock<String> = LazyLock::new(|| short_name(&hostname()));

/// What this machine calls itself, verbatim, or empty if the kernel will not say.
///
/// `libc::gethostname` rather than a crate: libc is already a direct dependency
/// (see `session.rs`'s fd handling), and this is one call.
///
/// ⚠ **A truncated name is not reported as an error by POSIX** — `gethostname`
/// may fill the buffer without a terminator and still return 0, which is why the
/// buffer is over-sized and the last byte is forced to nul rather than trusted.
/// The failure that guards against is a *silently shortened* hostname, which
/// would key home's table under a name that looks plausible and is wrong.
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

/// The name the rest of the fleet uses, given whatever the kernel returned.
///
/// Split out from [`hostname`] because this is the part with decisions in it and
/// the syscall is the part that cannot run twice the same way — a test can pin
/// the rules without owning a machine called anything in particular.
///
/// A search domain makes `mac-mini` and `mac-mini.local` the same machine, and
/// the short form is what `network.nix`, the `.vpn` names and home's existing
/// rows all use. Keying home's table on the long form would silently open a
/// second row for a host that already has one.
pub fn short_name(raw: &str) -> String {
    let short = raw.split('.').next().unwrap_or_default();
    if short.is_empty() {
        return UNKNOWN_HOST.to_string();
    }
    short.to_string()
}

/// What this console runs on, as it will be attributed.
///
/// Public so a test can say "attributed to this machine" without hard-coding the
/// name of the machine it happens to run on — which would pass here and fail on
/// every other box.
pub fn here() -> &'static str {
    &HERE
}

/// Never silently empty: a blank provenance reads as "no machine" rather than
/// "this machine would not say", and the two want different reactions.
const UNKNOWN_HOST: &str = "unknown host";

fn live(seen: Option<&Seen>, now_ms: i64) -> Option<Window> {
    let seen = seen?;
    Some(Window {
        // A fraction on the wire, a percentage on a screen — the same conversion
        // the CLI does on its way to a status line.
        pct: seen.utilization * 100.0,
        // Seconds there, milliseconds here: the conversion lives on the type,
        // because treating the CLI's epoch second as a millisecond puts every
        // reset in 1970 and reports every window as already turned over.
        resets_in_ms: seen
            .resets_at
            .map(|turns| turns.in_ms() - now_ms)
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
        // ⚠ **Never read back from the dashboard, however many it publishes.**
        // home's per-model figures are this console's own, pushed there by
        // `xinutec-infra/mac-mini/claude_usage_push.py` — reading them again would
        // be this console quoting itself through a round trip and calling the
        // answer corroboration.
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

/// How sessions are ranked when one of them must be asked what the account has
/// spent. Highest wins.
///
/// ⚠ **Idleness first, recency second — and this order is the whole point.** A
/// session answers `get_usage` from its own process's cached rate-limit headers,
/// so the one that spoke most recently holds the freshest figure; that is why
/// the console asks the most recent speaker. But a busy CLI does not answer a
/// control request until its turn ends, and "spoke most recently" is very nearly
/// "is working right now" — so the console was reliably asking the one session
/// least able to reply. Measured 2026-08-12: asked 2.0 s into a turn, answered at
/// 8.5 s (memview #817).
///
/// A session that has just finished a turn has a cache almost as fresh and
/// answers at once, which is the better trade. When every session is working
/// this still picks the freshest of them — the old behaviour, and better than
/// asking nobody at all.
pub fn asked_before(working: bool, last_heard: i64) -> (bool, i64) {
    (!working, last_heard)
}
