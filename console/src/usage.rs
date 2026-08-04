//! How much of the subscription is spent, and how long until it comes back.
//!
//! ⚠ **The console cannot measure this, and neither can anything on this Mac.**
//! The figure is Anthropic's own account-wide rate-limit utilisation — the one
//! `/usage` prints — and it reaches a machine by exactly one supported route:
//! Claude Code pipes it to a `statusLine` command on stdin. There is no API, no
//! CLI flag, and nothing on disk; the transcripts do not carry it (checked), and
//! `--output-format json` does not surface it. So this is not gathered here. It
//! is read from the home dashboard, which already collects it from that hook and
//! publishes the freshest reading across every machine — the same number, from
//! the same place, rather than a second and differently-wrong one.
//!
//! ⚠ **Which means it is only as fresh as the last interactive session
//! anywhere.** A status line belongs to a terminal, and the console's own
//! sessions are headless, so working *through the console* never refreshes this.
//! Hours-old readings are the normal case, not a fault — which is why age
//! travels with the number and why a window that has already reset reports no
//! figure at all rather than the one it had before it turned over.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tokio::sync::RwLock;

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
    pub five_hour: Window,
    pub seven_day: Window,
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

    /// The reading as of now, or nothing if none has ever arrived.
    ///
    /// ⚠ **Ages are computed here, against THIS machine's clock**, rather than
    /// sent as instants for the client to subtract from its own. A phone's clock
    /// drifts, and the one case that matters — "has this window already turned
    /// over?" — would then be answered differently on different screens.
    pub async fn reading(&self) -> Option<Reading> {
        let now = now_ms();
        self.latest.read().await.as_ref().map(|it| reading(it, now))
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
        five_hour: window(
            published.five_hour_pct,
            &published.five_hour_resets_at,
            now_ms,
        ),
        seven_day: window(
            published.seven_day_pct,
            &published.seven_day_resets_at,
            now_ms,
        ),
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
