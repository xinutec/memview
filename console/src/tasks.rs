//! The work a conversation is holding, read from the tasks service.
//!
//! ⚠ **This used to read `~/.claude/tasks/<session-id>/`, and that store was
//! deliberately emptied.** The CLI re-sent its whole contents as a
//! `task_reminder` attachment 1.75 times per message — 527 kB a turn on one
//! session, 93% of it a `description` the prompt never rendered — so the lists
//! were moved to a service at `tasks.xinutec.org` and the local files deleted.
//! Everything here kept reading the empty directory and reported nothing, which
//! looked exactly like conversations that keep no list.
//!
//! **Sessions, not repos.** The service files a task under a repo as well as an
//! assignee, but a repo is not a thing this console knows about: its unit is the
//! conversation, its cards are sessions, and the nearest it comes to a repo is
//! the directory a process happens to run in. So every read here is keyed on a
//! session id, which the console already has in hand for every row it draws.
//!
//! **What "this session's tasks" means here: the ones assigned to it.** Not the
//! ones its prompt sees — the prompt hook injects by *claimed repo*, which is
//! the repo vocabulary this console does not have. A card therefore says what
//! the conversation is holding, and a task nobody has been given shows on no
//! card at all, which is the truth about it.
//!
//! ⚠ **Reading is not being.** The service requires a caller to name the
//! conversation it speaks for, because a change filed against nobody is the one
//! thing its history must not contain. That is a rule about writes; this only
//! ever reads, and it is not a conversation. So it names itself — see
//! [`IDENTITY`] — rather than impersonating whichever session it is asking
//! about, which would put the console's reads in that session's name.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Where the service is. Overridable so a test can point at a local one, and so
/// a machine off the VPN could be pointed through a tunnel — the same variable
/// the `task` CLI and the prompt hook read.
fn service() -> String {
    std::env::var("TASKS_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| "https://tasks.xinutec.org".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Who the console says it is.
///
/// A constant, and deliberately not a session id. The alternative was to send
/// the id of whichever session a request is about, which reads as that
/// conversation asking — so the service's own record of who did what would show
/// a session making requests while it was asleep, or after it had ended.
const IDENTITY: &str = "agent-console";

/// The shared secret, from the environment or the file the Mac keeps it in —
/// the same two places the `task` CLI looks, in the same order.
///
/// Never on argv: a token in a command line is in every process listing on the
/// machine.
fn token() -> Option<String> {
    if let Ok(value) = std::env::var("TASKS_TOKEN")
        && !value.trim().is_empty()
    {
        return Some(value.trim().to_string());
    }
    let home = std::env::var("HOME").ok()?;
    std::fs::read_to_string(
        std::path::Path::new(&home)
            .join(".config")
            .join("tasks")
            .join("token"),
    )
    .ok()
    .map(|held| held.trim().to_string())
    .filter(|held| !held.is_empty())
}

/// One row of the list: what it is, and whether it is done.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Listed {
    /// The service's number, as a string — it is what a session calls a task in
    /// its own prose (`#418 done`) and what the sheet prints.
    #[serde(deserialize_with = "as_text")]
    pub id: String,
    pub subject: String,
    /// `open`, `doing` or `done`, in the service's own words rather than a
    /// boolean of our own: a third state exists and the client sorts on it.
    pub status: String,
    /// Whether there is prose behind it worth fetching. A task written as a
    /// one-line reminder has none, and offering to open an empty sheet is worse
    /// than not offering.
    #[serde(default)]
    pub detailed: bool,
}

/// The id arrives as a number and is used as a string everywhere above this.
fn as_text<'de, D: serde::Deserializer<'de>>(from: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(from)? {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    })
}

/// How much a conversation is holding, for a row drawn without opening it.
///
/// ⚠ **One number, where this was `open` over `total`.** The old store held a
/// session's whole list, so a total was the size of the job. Here a task is
/// assigned rather than owned, and everything ever assigned to a conversation —
/// including all of it finished — is a denominator that only grows and answers
/// no question anybody asks of a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Count {
    /// Anything not done. The difference between the two kinds of open is what
    /// the sheet is for.
    pub open: usize,
}

/// A session as the service's own front page draws it. Only the two fields this
/// console reads; the rest of the row is names and times it already has.
#[derive(Debug, Deserialize)]
struct Holding {
    id: String,
    open: i64,
}

/// The service, and the last answer it gave.
///
/// ⚠ **The cache is what makes a per-poll read affordable, and it is not an
/// optimisation.** The front page polls every five seconds, per client; the
/// service is on isis, across the VPN. Without this, a phone left open is a
/// request every five seconds to another machine for two numbers that change a
/// few times an hour — and a sleeping isis would stall the poll behind a
/// timeout. Measured against the live service, a request is 56-139 ms.
pub struct Tasks {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
    held: tokio::sync::RwLock<Option<Kept>>,
}

/// What was last had, and when.
struct Kept {
    counts: BTreeMap<String, Count>,
    at: Instant,
}

impl Default for Tasks {
    fn default() -> Self {
        Self::new()
    }
}

impl Tasks {
    /// How long an answer is served without asking again.
    const TTL: Duration = Duration::from_secs(30);

    /// Hard ceiling on a request. An order above the measured 56-139 ms and far
    /// below anything a person waiting for a page would notice.
    const TIMEOUT: Duration = Duration::from_secs(2);

    pub fn new() -> Self {
        Self::at(service())
    }

    /// A reader pointed at one service.
    ///
    /// ⚠ **The address is an argument, not read from the environment here.** It
    /// was, and the tests set `TASKS_URL` before building each reader — which is
    /// process-wide, so tests running in parallel clobbered each other's stub
    /// and five of seven failed against the wrong server. A reader that is told
    /// where to look can be built twice in one process.
    pub fn at(base: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Self::TIMEOUT)
                .build()
                .unwrap_or_default(),
            base: base.into().trim_end_matches('/').to_string(),
            token: token(),
            held: tokio::sync::RwLock::new(None),
        }
    }

    fn asking(&self, path: &str) -> reqwest::RequestBuilder {
        let mut request = self.http.get(format!("{}{path}", self.base));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        request.header("X-Session-Id", IDENTITY)
    }

    /// Every conversation holding something, and how much.
    ///
    /// One request for the whole page rather than one per row: the service
    /// sweeps its sessions in a single query, exactly as this used to sweep one
    /// directory. A session holding nothing is **absent** rather than zero,
    /// which is the rule the client draws by — no work is not an empty list.
    ///
    /// Whatever goes wrong — isis asleep, tunnel down, token missing, service
    /// mid-deploy — the last known answer is served, and an empty map only when
    /// there has never been one. Stale beats blocking, and both beat failing.
    pub async fn sweep(&self) -> BTreeMap<String, Count> {
        if let Some(kept) = self.held.read().await.as_ref()
            && kept.at.elapsed() < Self::TTL
        {
            return kept.counts.clone();
        }
        match self.ask().await {
            Ok(counts) => {
                *self.held.write().await = Some(Kept {
                    counts: counts.clone(),
                    at: Instant::now(),
                });
                counts
            }
            Err(failure) => {
                // Logged at debug: a console left running through a reboot of
                // isis would otherwise fill the log with one line every thirty
                // seconds, describing something already visible as a list that
                // stopped moving.
                tracing::debug!("the tasks service did not answer: {failure}");
                self.held
                    .read()
                    .await
                    .as_ref()
                    .map(|kept| kept.counts.clone())
                    .unwrap_or_default()
            }
        }
    }

    async fn ask(&self) -> reqwest::Result<BTreeMap<String, Count>> {
        let holding: Vec<Holding> = self
            .asking("/api/sessions")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        Ok(holding
            .into_iter()
            .filter(|row| row.open > 0)
            .map(|row| {
                let open = usize::try_from(row.open).unwrap_or(0);
                (row.id, Count { open })
            })
            .collect())
    }

    /// One conversation's tasks, oldest first, without their prose.
    ///
    /// ⚠ **The list and a task's own words are two requests, on purpose.** A
    /// body is not a label: these run to several kilobytes of written-up result,
    /// and sending them with the list is a megabyte onto a phone to draw forty
    /// subjects.
    pub async fn listed(&self, session: &str) -> Vec<Listed> {
        let asked = self
            .asking("/api/tasks")
            .query(&[("session", session), ("done", "true")])
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        match asked {
            Ok(answer) => answer.json().await.unwrap_or_else(|failure| {
                tracing::warn!("unreadable task list for {session}: {failure}");
                Vec::new()
            }),
            Err(failure) => {
                tracing::warn!("no task list for {session}: {failure}");
                Vec::new()
            }
        }
    }

    /// One task's prose, fetched when a row is opened and not before.
    ///
    /// `None` rather than an empty string for a task that has none, so the sheet
    /// can tell "nothing written" from "not fetched yet".
    pub async fn detail(&self, id: &str) -> Option<String> {
        #[derive(Deserialize)]
        struct Body {
            #[serde(default)]
            body: String,
        }
        let asked = self
            .asking(&format!("/api/tasks/{id}"))
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .ok()?;
        let detail: Body = asked.json().await.ok()?;
        Some(detail.body).filter(|prose| !prose.trim().is_empty())
    }
}
