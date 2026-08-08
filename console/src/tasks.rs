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
/// ⚠ **`total` is what is assigned *now*, not what ever was.** A denominator
/// counting everything ever handed to a conversation only grows, so finishing
/// work would make the fraction worse — which is why this was one number for a
/// while. The service counts current assignment (tasks#636), so a task handed
/// on leaves both halves and `3/47` says what it looks like it says.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Count {
    /// Anything not done. The difference between the two kinds of open is what
    /// the sheet is for.
    pub open: usize,
    /// Open and finished together. Never smaller than [`Self::open`].
    pub total: usize,
}

/// Somebody holding tasks who is not one of this console's conversations:
/// Pippijn, and the unassigned pile.
///
/// Kept rather than filtered away because a card reading `12/34` means one thing
/// beside "Pippijn is holding 4" and another beside "23 are in the pile" — and
/// the pile is the one nothing else on this page can show, since it belongs to
/// no session and therefore appears on no card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Held {
    /// What to call them. The service's own word — `Pippijn`, `nobody`.
    pub name: String,
    pub open: usize,
    pub total: usize,
}

/// Everything the sweep learnt, in one request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sweep {
    /// By session id, for the cards.
    pub sessions: BTreeMap<String, Count>,
    /// The holders who are not sessions, in the order the service put them.
    pub elsewhere: Vec<Held>,
}

/// One holder as the service reports it.
///
/// ⚠ **`id` is absent on the `nobody` row** — the pile is not anybody, so it has
/// no id to give. A non-optional field here fails the whole sweep on that one
/// row, and the sessions would go with it.
#[derive(Debug, Deserialize)]
struct Holder {
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    open: i64,
    total: i64,
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
        // ⚠ **Here, and not only in `main`.** Building a TLS client with no
        // process-wide crypto provider panics inside reqwest — `No provider
        // set`, at run time, and `unwrap_or_default()` below is no escape
        // because the default client panics identically. `main.rs` installs one
        // before anything asks; a test that builds a [`crate::roster::Roster`]
        // does not, and 32 of them died in here rather than in the code they
        // were testing. Idempotent: already-installed means somebody got there
        // first, which is the ordinary case in the binary.
        let _ = rustls::crypto::ring::default_provider().install_default();
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

    /// Everybody holding something, and how much.
    ///
    /// One request for the whole page rather than one per row: the service
    /// counts every holder in a single query, exactly as this used to sweep one
    /// directory. A conversation holding nothing is **absent** rather than zero,
    /// which is the rule the client draws by — no work is not an empty list.
    ///
    /// ⚠ **`/api/holders`, not `/api/sessions`.** The session table has the
    /// names and the times and feeds the assignee picker; the tally is its own
    /// endpoint so that two of them cannot drift, and because it can answer for
    /// the person and the pile, which a table of sessions structurally cannot.
    ///
    /// Whatever goes wrong — isis asleep, tunnel down, token missing, service
    /// mid-deploy — the last known answer is served, and an empty one only when
    /// there has never been one. Stale beats blocking, and both beat failing.
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
                // Logged at debug: a console left running through a reboot of
                // isis would otherwise fill the log with one line every thirty
                // seconds, describing something already visible as a list that
                // stopped moving.
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
        let holders: Vec<Holder> = self
            .asking("/api/holders")
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        let mut swept = Sweep::default();
        for row in holders {
            let open = usize::try_from(row.open).unwrap_or(0);
            let total = usize::try_from(row.total).unwrap_or(0);
            // ⚠ **Nothing ever assigned, rather than nothing open.** The rule the
            // client draws by is "absent means this conversation was never
            // handed anything", and until there was a total that could only be
            // approximated by `open > 0` — which hid a session that had
            // *finished* its list, the one case where the row is worth drawing.
            // `0/366` is a good day; no row at all is a different fact.
            if total == 0 {
                continue;
            }
            match (row.kind.as_str(), row.id) {
                ("session", Some(id)) => {
                    swept.sessions.insert(id, Count { open, total });
                }
                // Person and pile keep the service's order: it decides who is
                // loaded, in one place, so `task sessions`, the app and this
                // cannot disagree about it.
                _ => swept.elsewhere.push(Held {
                    name: row.name.unwrap_or_else(|| row.kind.clone()),
                    open,
                    total,
                }),
            }
        }
        Ok(swept)
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
