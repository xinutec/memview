//! The work a conversation is holding, read from the tasks service.
//!
//! Not `~/.claude/tasks/`, which is deliberately empty: the CLI re-sent that
//! store's whole contents with every message, so the lists moved to a service.
//!
//! "This session's tasks" means the ones ASSIGNED to it, not the ones its prompt
//! sees. And reading is not being: the service makes writers name the conversation
//! they speak for, but this only reads, so it names itself — see [`IDENTITY`].

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Where the service is. Overridable for a test, and the same variable the `task`
/// CLI and the prompt hook read.
fn service() -> String {
    std::env::var("TASKS_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| "https://tasks.xinutec.org".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Who the console says it is. A constant, not a session id: the service's own
/// record would otherwise show a session making requests while asleep.
const IDENTITY: &str = "agent-console";

/// The shared secret, from the environment or the file — the two places the `task`
/// CLI looks, in the same order. Never on argv.
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
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Task {
    /// The service's number, as a string: what a session calls a task in its prose
    /// (`#418 done`) and what the sheet prints.
    #[serde(deserialize_with = "as_text")]
    pub id: String,
    pub subject: String,
    /// `open`, `doing` or `done`, in the service's own words: a third state exists
    /// and the client sorts on it.
    pub status: String,
    /// Whether there is prose behind it worth fetching.
    #[serde(default)]
    pub detailed: bool,
    /// How urgent, in the service's own words — `P0` to `P4`. Absent on most rows,
    /// and absence is not a sixth level: an unranked task sorts where `P2` does.
    /// Nothing here sorts on it; `repo::list` is the service's only ordering. A string,
    /// so a level invented later is news to draw, not a parse failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub priority: Option<String>,
    /// The day it has to be done by, `YYYY-MM-DD`. Never sorted on: a deadline is
    /// evidence for a priority, not a competing answer to what-next.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub due: Option<String>,
    /// Whether that day has passed. Server-decided, from the database's clock, and NOT
    /// recomputed from [`Self::due`] on a phone in another timezone.
    #[serde(default)]
    pub overdue: bool,
    /// Which tasks this one is waiting for, by number. Absent when empty.
    #[serde(default, deserialize_with = "as_texts")]
    pub blocked_on: Vec<String>,
    /// Whether it is actually still waiting. Server-decided, and NOT `blocked_on`
    /// being non-empty: the link survives its blocker closing, as a record.
    #[serde(default)]
    pub blocked: bool,
}

/// The ids arrive as numbers and are used as strings everywhere above this.
fn as_texts<'de, D: serde::Deserializer<'de>>(from: D) -> Result<Vec<String>, D::Error> {
    Ok(Vec::<serde_json::Value>::deserialize(from)?
        .into_iter()
        .map(|held| match held {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        })
        .collect())
}

/// The id arrives as a number and is used as a string everywhere above this.
fn as_text<'de, D: serde::Deserializer<'de>>(from: D) -> Result<String, D::Error> {
    Ok(match serde_json::Value::deserialize(from)? {
        serde_json::Value::String(text) => text,
        other => other.to_string(),
    })
}

/// How much a conversation is holding, for a row drawn without opening it. `total`
/// is what is assigned NOW, not what ever was, so finishing work does not make the
/// fraction worse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TaskCount {
    /// Anything not done. The difference between the two kinds of open is what
    /// the sheet is for.
    pub open: usize,
    /// Open and finished together. Never smaller than [`Self::open`].
    pub total: usize,
    /// How many are still in the built-in store this replaced — see [`strays`].
    #[serde(default)]
    pub stray: usize,
}

/// Somebody holding tasks who is not one of this console's conversations —
/// Pippijn, and the unassigned pile, which appears on no card otherwise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Holder {
    /// What to call them. The service's own word — `Pippijn`, `nobody`.
    pub name: String,
    pub open: usize,
    pub total: usize,
}

/// Everything the sweep learnt, in one request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Sweep {
    /// By session id, for the cards.
    pub sessions: BTreeMap<String, TaskCount>,
    /// The holders who are not sessions, in the order the service put them.
    pub elsewhere: Vec<Holder>,
}

/// What each conversation has left in `~/.claude/tasks/<session-id>/`, the
/// built-in store the service replaced. Every file there is re-sent to its session
/// with every message, so a number here says a migration was never cleaned up, or
/// something is still filing into the expensive store.
///
/// Counts files rather than reading them; `.lock` and `.highwatermark` are the
/// CLI's. Unreadable is nothing rather than an error: a console running anywhere
/// but this Mac cannot see the old store.
fn strays(store: &std::path::Path) -> BTreeMap<String, usize> {
    let Ok(sessions) = std::fs::read_dir(store) else {
        return BTreeMap::new();
    };
    sessions
        .flatten()
        .filter_map(|session| {
            let id = session.file_name().to_str()?.to_string();
            let left = std::fs::read_dir(session.path())
                .ok()?
                .flatten()
                .filter(|task| task.path().extension().is_some_and(|kind| kind == "json"))
                .count();
            (left > 0).then_some((id, left))
        })
        .collect()
}

/// One holder as the service reports it. `id` is absent on the `nobody` row.
#[derive(Debug, Deserialize)]
struct HolderRow {
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    open: i64,
    total: i64,
}

/// The service, and the last answer it gave. The cache is what makes a per-poll
/// read affordable: the front page polls every five seconds, the service is across
/// the VPN, and a sleeping isis would stall the poll behind a timeout.
pub struct Tasks {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
    /// The built-in store to count leftovers in — see [`strays`]. A path rather than
    /// `$HOME` read on the spot, so a test can point at a fixture.
    store: std::path::PathBuf,
    held: tokio::sync::RwLock<Option<Kept>>,
}

/// What was last had, and when.
struct Kept {
    swept: Sweep,
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

    /// Hard ceiling on a request, an order above the measured 56–139 ms.
    const TIMEOUT: Duration = Duration::from_secs(2);

    pub fn new() -> Self {
        Self::at(service())
    }

    /// A reader pointed at one service. The address is an argument, not the
    /// environment: tests setting `TASKS_URL` clobbered each other's stub.
    pub fn at(base: impl Into<String>) -> Self {
        // Here and not only in `main`: building a TLS client with no process-wide crypto
        // provider panics inside reqwest, and a test that builds a Roster has run no
        // `main`. Idempotent.
        let _ = rustls::crypto::ring::default_provider().install_default();
        Self {
            http: reqwest::Client::builder()
                .timeout(Self::TIMEOUT)
                .build()
                .unwrap_or_default(),
            base: base.into().trim_end_matches('/').to_string(),
            token: token(),
            store: std::env::var_os("HOME")
                .map(|home| std::path::Path::new(&home).join(".claude/tasks"))
                .unwrap_or_default(),
            held: tokio::sync::RwLock::new(None),
        }
    }

    /// TaskCount leftovers in `store` instead of the one beside `$HOME`.
    pub fn counting(mut self, store: impl Into<std::path::PathBuf>) -> Self {
        self.store = store.into();
        self
    }

    fn asking(&self, path: &str) -> reqwest::RequestBuilder {
        let mut request = self.http.get(format!("{}{path}", self.base));
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        request.header("X-Session-Id", IDENTITY)
    }

    /// Everybody holding something, and how much. One request for the whole page. A
    /// conversation holding nothing is ABSENT rather than zero, which is the rule the
    /// client draws by.
    ///
    /// `/api/holders`, not `/api/sessions`: the tally is its own endpoint so the two
    /// cannot drift, and it can answer for the person and the pile.
    ///
    /// Whatever goes wrong, the last known answer is served, and an empty one only
    /// when there has never been one.
    pub async fn sweep(&self) -> Sweep {
        if let Some(kept) = self.held.read().await.as_ref()
            && kept.at.elapsed() < Self::TTL
        {
            return kept.swept.clone();
        }
        match self.ask().await {
            Ok(swept) => {
                *self.held.write().await = Some(Kept {
                    swept: swept.clone(),
                    at: Instant::now(),
                });
                swept
            }
            Err(failure) => {
                // Debug: a console running through a reboot of isis would otherwise say this
                // every thirty seconds.
                tracing::debug!("the tasks service did not answer: {failure}");
                self.held
                    .read()
                    .await
                    .as_ref()
                    .map(|kept| kept.swept.clone())
                    .unwrap_or_default()
            }
        }
    }

    async fn ask(&self) -> reqwest::Result<Sweep> {
        let holders: Vec<HolderRow> = self
            .asking("/api/holders")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        // Off the executor: a handful of `readdir`s is not quick on a cold cache, and
        // this runs behind the front page's poll.
        let store = self.store.clone();
        let mut left = tokio::task::spawn_blocking(move || strays(&store))
            .await
            .unwrap_or_default();

        let mut swept = Sweep::default();
        for row in holders {
            let open = usize::try_from(row.open).unwrap_or(0);
            let total = usize::try_from(row.total).unwrap_or(0);
            match (row.kind.as_str(), row.id) {
                ("session", Some(id)) => {
                    let stray = left.remove(&id).unwrap_or(0);
                    // Nothing ever assigned, rather than nothing open: `0/366` is a good day, and
                    // no row at all is a different fact.
                    if total == 0 && stray == 0 {
                        continue;
                    }
                    swept.sessions.insert(id, TaskCount { open, total, stray });
                }
                // Person and pile keep the service's order, so `task sessions`, the app and this
                // cannot disagree.
                _ if total > 0 => swept.elsewhere.push(Holder {
                    name: row.name.unwrap_or_else(|| row.kind.clone()),
                    open,
                    total,
                }),
                _ => {}
            }
        }
        // Leftovers from a session the service has never heard of: no holder row by
        // definition, so a loop over holders alone cannot find it.
        for (id, stray) in left {
            swept.sessions.insert(
                id,
                TaskCount {
                    open: 0,
                    total: 0,
                    stray,
                },
            );
        }
        Ok(swept)
    }

    /// One conversation's tasks, oldest first, without their prose. The list and a
    /// task's words are two requests: bodies run to kilobytes each.
    pub async fn listed(&self, session: &str) -> Vec<Task> {
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

    /// One task's prose, fetched when a row is opened. `None` rather than an empty
    /// string, so the sheet can tell "nothing written" from "not fetched yet".
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
