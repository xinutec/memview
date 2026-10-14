//! The work a conversation is holding, read from the tasks service.
//!
//! A session's tasks are the ones assigned to it. The console only reads, so it
//! names itself — [`IDENTITY`].

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Where the service is — the variable the `task` CLI reads too.
fn service() -> String {
    std::env::var("TASKS_URL")
        .ok()
        .filter(|url| !url.trim().is_empty())
        .unwrap_or_else(|| "https://tasks.xinutec.org".to_string())
        .trim_end_matches('/')
        .to_string()
}

/// Who the console says it is — not a session, which may be asleep.
const IDENTITY: &str = "agent-console";

/// The shared secret, from where the `task` CLI looks. Never on argv.
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

/// Where a task stands, in the service's own words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub enum Status {
    Open,
    Doing,
    Done,
    /// Closed without being done: overtaken, obsolete, or decided against.
    Dropped,
    Unknown(String),
}

impl crate::named::Named for Status {
    fn unknown(name: String) -> Self {
        Status::Unknown(name)
    }
}

/// One row of the list: what it is, and whether it is done.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Task {
    /// The service's number, as a string.
    #[serde(deserialize_with = "as_text")]
    pub id: String,
    pub subject: String,
    #[serde(deserialize_with = "crate::named::by_name")]
    pub status: Status,
    /// Whether there is prose behind it worth fetching.
    #[serde(default)]
    pub detailed: bool,
    /// How urgent, `P0` to `P4`, as the service says it. A string, so a new level
    /// draws rather than fails.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub priority: Option<String>,
    /// The day it has to be done by, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub due: Option<String>,
    /// Whether that day has passed, by the server's clock.
    #[serde(default)]
    pub overdue: bool,
    /// Which tasks this one is waiting for, by number. Absent when empty.
    #[serde(default, deserialize_with = "as_texts")]
    pub blocked_on: Vec<String>,
    /// Whether it still waits — the link outlives a closed blocker.
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

/// How much a conversation is holding now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct TaskCount {
    /// Anything not done.
    pub open: usize,
    /// Open and finished together. Never smaller than [`Self::open`].
    pub total: usize,
    /// How many are still in the CLI's built-in store — see [`strays`].
    #[serde(default)]
    pub stray: usize,
}

/// A holder that is not a conversation: the user, or the unassigned pile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Holder {
    /// What to call them. The service's own word — the account, or `nobody`.
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

/// How many files each conversation has in the CLI's built-in task store, each
/// re-sent with every message — pure cost.
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

/// The service, and its last answer, cached so a sleeping server cannot stall a poll.
pub struct Tasks {
    http: reqwest::Client,
    base: String,
    token: Option<String>,
    /// The built-in store to count leftovers in — see [`strays`].
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

    /// Hard ceiling on a request, an order of magnitude above a normal one.
    const TIMEOUT: Duration = Duration::from_secs(2);

    pub fn new() -> Self {
        Self::at(service())
    }

    /// A reader pointed at one service, by argument so parallel tests do not clash.
    pub fn at(base: impl Into<String>) -> Self {
        // Also here, for tests that run no `main`.
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

    /// Everybody holding something, and how much, in one request. On failure, the
    /// last known answer.
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
                // Debug: it would repeat through a reboot of isis.
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
        // Off the executor: behind the front page's poll.
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
                    // Nothing ever assigned: no row, unlike all done.
                    if total == 0 && stray == 0 {
                        continue;
                    }
                    swept.sessions.insert(id, TaskCount { open, total, stray });
                }
                // The service's order.
                _ if total > 0 => swept.elsewhere.push(Holder {
                    name: row.name.unwrap_or_else(|| row.kind.clone()),
                    open,
                    total,
                }),
                _ => {}
            }
        }
        // Leftovers from sessions the service has never heard of.
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

    /// One conversation's tasks, oldest first, without their prose.
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

    /// One task's prose; `None` when nothing is written.
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
