//! JSON API. All corpus reads admit the owner or a share-token holder;
//! share management is owner-only.

use axum::Json;
use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::access::{OwnerOnly, ReadAccess, Viewer};
use crate::error::AppError;
use crate::share::build_share_url;
use crate::state::AppState;
use crate::store::{Corpus, Graph, MemoryMeta, render_markdown};

fn load_corpus(app: &AppState) -> Result<Corpus, AppError> {
    Ok(Corpus::load(&app.cfg.memory_dir)?)
}

/// GET /api/reading — the corpus survey: what the reader makes of the fleet's
/// shell, and what it admits it cannot.
///
/// ⚠ **Owner-only, like the timeline and the evidence.** It names paths and
/// command names from Pippijn's machines; a share-token holder gets memories,
/// not a map of the fleet's filesystem.
///
/// 404 rather than an empty body when nothing has been mined: the view must be
/// able to say "not mined yet" and mean it, which it cannot do from a `CorpusRead`
/// full of zeroes.
pub async fn reading(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
) -> Result<Json<reader::reading::CorpusRead>, AppError> {
    let summary = app.reading().ok_or(AppError::NotFound)?;
    Ok(Json((*summary).clone()))
}

/// GET /api/me
pub async fn me(State(app): State<AppState>, ReadAccess(viewer): ReadAccess) -> Json<Value> {
    let auth_enabled = app.cfg.auth.is_some();
    match viewer {
        Viewer::Owner(user) => Json(json!({
            "user_id": user.user_id,
            "display_name": user.display_name,
            "shared": false,
            "auth_enabled": auth_enabled,
        })),
        Viewer::Shared => Json(json!({
            "shared": true,
            "auth_enabled": auth_enabled,
        })),
    }
}

/// GET /api/index — MEMORY.md rendered, links rewritten to /m/<name>.
pub async fn index(
    State(app): State<AppState>,
    ReadAccess(_): ReadAccess,
) -> Result<Json<Value>, AppError> {
    let corpus = load_corpus(&app)?;
    let md = corpus.index_md.ok_or(AppError::NotFound)?;
    Ok(Json(json!({
        "html": render_markdown(&md)?,
        "count": corpus.docs.len(),
    })))
}

/// GET /api/memories — every memory's metadata.
pub async fn memories(
    State(app): State<AppState>,
    ReadAccess(_): ReadAccess,
) -> Result<Json<Vec<MemoryMeta>>, AppError> {
    Ok(Json(load_corpus(&app)?.list()))
}

/// Who wrote a memory: the session its frontmatter names, and the agent that
/// session belongs to when the roster still knows.
///
/// The name is the point — a uuid answers "which session" without answering
/// "which of my agents". It stays optional because Claude Code prunes its own
/// old sessions, so a memory routinely outlives the transcript that wrote it;
/// the id is still shown then, since "written by a session I no longer have"
/// is a truer answer than silence.
#[derive(Serialize)]
pub struct Origin {
    session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<String>,
}

#[derive(Serialize)]
pub struct MemoryPage {
    #[serde(flatten)]
    meta: MemoryMeta,
    html: String,
    backlinks: Vec<MemoryMeta>,
    outlinks: Vec<MemoryMeta>,
    /// Wikilink targets not written yet — surfaced, not hidden; a dangling
    /// link marks something worth writing.
    dangling: Vec<String>,
    /// Owner-only, and absent for a share-link recipient. Same reason
    /// `/api/agents` is owner-only: a link to one memory must not also hand
    /// over who is working on what. Absent here means "not shown to you",
    /// which is indistinguishable from "the memory declares none" — deliberately,
    /// since the count of memories an agent wrote is itself part of the roster.
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<Origin>,
}

/// The agent roster, or an empty one when nothing has been mined.
///
/// Absent is normal — a fresh checkout or CI has no transcripts — and it
/// degrades to showing the bare session id rather than to an error.
fn roster(app: &AppState) -> crate::agents::Agents {
    app.cfg
        .agents_file
        .as_deref()
        .and_then(|p| crate::agents::Agents::load(std::path::Path::new(p)))
        .unwrap_or_default()
}

/// GET /api/memory/{name}
pub async fn memory(
    State(app): State<AppState>,
    ReadAccess(viewer): ReadAccess,
    Path(name): Path<String>,
) -> Result<Json<MemoryPage>, AppError> {
    let corpus = load_corpus(&app)?;
    let doc = corpus.get(&name).ok_or(AppError::NotFound)?;
    let (outlinks, dangling) = corpus.outlinks(doc);
    // Resolved only for the owner, and only when the memory declares one. The
    // roster is loaded inside the match so a share request does not pay to read
    // an artefact it may not see.
    let origin = match viewer {
        Viewer::Owner(_) => doc.origin_session.as_ref().map(|session| Origin {
            agent: roster(&app).name_of_session(session).map(str::to_string),
            session: session.clone(),
        }),
        Viewer::Shared => None,
    };
    Ok(Json(MemoryPage {
        meta: doc.meta.clone(),
        html: render_markdown(&doc.body)?,
        backlinks: corpus.backlinks(&name),
        outlinks,
        dangling,
        origin,
    }))
}

/// GET /api/graph — the corpus as a link graph, for the 3D view. One payload
/// for the whole graph: at corpus scale (hundreds of nodes, ~2 edges each) it
/// is tens of KB, and a layout needs every node before it can place any.
pub async fn graph(
    State(app): State<AppState>,
    ReadAccess(_): ReadAccess,
) -> Result<Json<Graph>, AppError> {
    let mut graph = load_corpus(&app)?.graph();
    if let Some(path) = &app.cfg.couse_file
        && let Some(couse) = crate::couse::CoUse::load(std::path::Path::new(path))
    {
        graph.usage = couse.usage;
        graph.affinities = couse.pairs;
    }
    Ok(Json(graph))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
}

/// The mined per-memory usage, or an empty map when nothing has been mined.
///
/// Absent is a normal state, not a failure: a machine with no transcripts (CI,
/// or a fresh checkout) has no artefact, and search must still work there —
/// only the ordering among comparable answers changes.
fn usage_of(app: &AppState) -> BTreeMap<String, crate::couse::Usage> {
    app.cfg
        .couse_file
        .as_deref()
        .and_then(|p| crate::couse::CoUse::load(std::path::Path::new(p)))
        .map(|c| c.usage)
        .unwrap_or_default()
}

/// GET /api/search?q=
pub async fn search(
    State(app): State<AppState>,
    ReadAccess(_): ReadAccess,
    Query(query): Query<SearchQuery>,
) -> Result<Json<crate::store::SearchResult>, AppError> {
    let corpus = load_corpus(&app)?;
    Ok(Json(corpus.search(&query.q, &usage_of(&app))))
}

/// GET /api/agents — which named session works where (owner only).
///
/// Owner-only, and not because the numbers are secret — they are counts, the
/// same shape of derived signal the graph already serves. A share token is a
/// deliberately public surface, though, and a link to one memory must not also
/// hand over the roster of what is being worked on and by whom. That is the
/// shape of the work, not a memory.
pub async fn agents(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
) -> Result<Json<crate::agents::Agents>, AppError> {
    Ok(Json(roster(&app)))
}

/// GET /api/work?q= — who has been changing the files a query names (owner only).
///
/// The companion to `/api/search`, which searches what was *written down*. This
/// searches what was *worked on*, and they answer different questions: a subtree
/// nobody has documented still has somebody who knows it.
///
/// Owner-only for the same reason as [`agents`] — it is the roster, sliced.
pub async fn work(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<crate::agents::WorkMatch>>, AppError> {
    Ok(Json(roster(&app).who_works_on(&query.q)))
}

/// What to show of the timeline. Every field narrows it; none is required.
#[derive(Debug, Deserialize)]
pub struct DoingQuery {
    pub agent: Option<String>,
    pub project: Option<String>,
    pub kind: Option<String>,
    /// Rows older than this minute, for paging backwards through the history.
    pub before: Option<i64>,
    pub limit: Option<usize>,
}

/// One thing an agent did, with its dictionaries resolved.
#[derive(Debug, Serialize)]
pub struct Moment {
    pub at: i64,
    pub agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    pub kind: String,
    pub n: u32,
    pub verdict: reader::doing::Verdict,
    /// How many effects `/api/effects` would return for this row.
    ///
    /// ⚠ **On the row, because 12.6% of rows have NONE.** Measured over the
    /// live artefact: 58,644 of 466,951 moments have no effect at all — a
    /// `test` or a `build` minute need not touch a file — and 27 of the newest
    /// 200 are empty. Without this the only way to find that out is to open one
    /// and read "nothing was recorded", which is a tap spent to learn there was
    /// nothing to learn. It also says which turns are worth opening: the median
    /// row carries 10 and the 90th percentile carries 85.
    pub effects: u32,
    /// Which instruction this was part of — an index into [`Timeline::episodes`]
    /// for the page, not the artefact's own numbering.
    ///
    /// ⚠ **An id and not the episode itself**, because a stretch of one
    /// instruction is commonly dozens of rows and repeating its bracket on each
    /// would be most of the response.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode: Option<usize>,
}

/// One instruction, as much of it as the page can see.
///
/// ⚠ **Not `Episode`, because `reader::doing::Episode` is already that** — and
/// the wire-mirror check pairs a TypeScript interface with a Rust type BY NAME.
/// Two candidates and it chose the artefact's, then reported every field of
/// this one as drift.
#[derive(Debug, Serialize)]
pub struct TimelineEpisode {
    pub agent: String,
    /// The minute of its first row — which may be older than anything on this
    /// page, and is the point: an episode says how large the stretch was, not
    /// how much of it fitted.
    pub at: i64,
    pub until: i64,
    pub n: u32,
}

/// A slice of the timeline, and what the whole of the filtered range contains.
#[derive(Debug, Serialize)]
pub struct Timeline {
    pub moments: Vec<Moment>,
    /// The instructions the moments above were carried out under, referenced by
    /// `Moment::episode`. Only those the page touches.
    pub episodes: Vec<TimelineEpisode>,
    /// Kinds of work in the filtered range, biggest first — the shape of the
    /// answer, which a page of two hundred rows cannot show.
    pub summary: Vec<(String, usize)>,
    pub total: usize,
    pub failed: usize,
}

/// How many moments one request may take. A page, not a download: the artefact
/// is two hundred thousand rows and nothing renders that.
const PAGE: usize = 200;

/// GET /api/doing — what the sessions did, newest first (owner only).
///
/// Owner-only for the same reason as the roster, and more so: this is the shape
/// of the work over time. Derived throughout — no command line, no prompt and
/// no output text exists in the artefact to serve.
pub async fn doing(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
    Query(query): Query<DoingQuery>,
) -> Result<Json<Timeline>, AppError> {
    let log = app.doing();
    // Counted with the artefact rather than here — see `effects_and_counts`.
    let (effects_log, counts) = app.effects_and_counts();
    // ⚠ **The two artefacts index their agents separately**, so the join is by
    // name — built ONCE for the whole timeline rather than per row. The first
    // draft resolved it inside the loop and carried this same comment, which was
    // then a false one: a page of 200 rows did 200 linear searches.
    let effects_agent: Vec<Option<u32>> = log
        .agents
        .iter()
        .map(|name| {
            effects_log
                .agents
                .iter()
                .position(|other| other.eq_ignore_ascii_case(name))
                .map(|at| at as u32)
        })
        .collect();
    let at = |names: &[String], want: &Option<String>| -> Option<Option<u32>> {
        match want {
            None => Some(None),
            Some(want) => names
                .iter()
                .position(|name| name.eq_ignore_ascii_case(want))
                .map(|at| Some(at as u32)),
        }
    };
    // A filter naming something the corpus has never seen matches nothing,
    // rather than matching everything — the difference between "no such agent"
    // and "here is the whole history".
    let (Some(agent), Some(project), Some(kind)) = (
        at(&log.agents, &query.agent),
        at(&log.projects, &query.project),
        at(&log.kinds, &query.kind),
    ) else {
        return Ok(Json(Timeline {
            moments: Vec::new(),
            episodes: Vec::new(),
            summary: Vec::new(),
            total: 0,
            failed: 0,
        }));
    };
    let matching = log.rows.iter().rev().filter(|row| {
        agent.is_none_or(|a| row.a == a)
            && project.is_none_or(|p| row.p == Some(p))
            && kind.is_none_or(|k| row.k == k)
            && query.before.is_none_or(|before| row.t < before)
    });
    let mut summary: BTreeMap<&str, usize> = BTreeMap::new();
    let mut total = 0usize;
    let mut failed = 0usize;
    let mut moments = Vec::new();
    // The episodes this page touches, numbered for the page. A stretch of one
    // instruction is commonly dozens of rows, so the same handful repeat.
    let mut episodes: Vec<TimelineEpisode> = Vec::new();
    let mut episode_at: BTreeMap<u32, usize> = BTreeMap::new();
    let limit = query.limit.unwrap_or(PAGE).min(PAGE);
    for row in matching {
        total += 1;
        failed += usize::from(row.v == reader::doing::Verdict::Failed);
        let kind = log
            .kinds
            .get(row.k as usize)
            .map(String::as_str)
            .unwrap_or("");
        *summary.entry(kind).or_default() += row.n as usize;
        if moments.len() < limit {
            let agent = log.agents.get(row.a as usize).cloned().unwrap_or_default();
            let effects = effects_agent
                .get(row.a as usize)
                .copied()
                .flatten()
                .and_then(|a| counts.get(&(a, row.t)).copied())
                .unwrap_or(0);
            let episode = row.e.and_then(|at| {
                let known = episode_at.get(&at).copied();
                known.or_else(|| {
                    let episode = log.episodes.get(at as usize)?;
                    let agent = log
                        .agents
                        .get(episode.a as usize)
                        .cloned()
                        .unwrap_or_default();
                    episodes.push(TimelineEpisode {
                        agent,
                        at: episode.t,
                        until: episode.until,
                        n: episode.n,
                    });
                    let here = episodes.len() - 1;
                    episode_at.insert(at, here);
                    Some(here)
                })
            });
            moments.push(Moment {
                at: row.t,
                agent,
                project: row.p.and_then(|p| log.projects.get(p as usize).cloned()),
                host: row.h.and_then(|h| log.hosts.get(h as usize).cloned()),
                kind: kind.to_string(),
                n: row.n,
                verdict: row.v,
                effects,
                episode,
            });
        }
    }
    let mut summary: Vec<(String, usize)> = summary
        .into_iter()
        .map(|(kind, n)| (kind.to_string(), n))
        .collect();
    summary.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(Json(Timeline {
        moments,
        episodes,
        summary,
        total,
        failed,
    }))
}

/// Which turn's effects to show. `agent` and `at` together are one timeline row.
#[derive(Debug, Deserialize)]
pub struct EffectsQuery {
    pub agent: Option<String>,
    /// The minute a [`Moment`] carries. Given alone it is the whole fleet's
    /// minute; given with `agent` it is one turn.
    pub at: Option<i64>,
    /// Substring of the path, for "what happened to this file" — the other
    /// direction through the same rows.
    pub path: Option<String>,
    pub limit: Option<usize>,
}

/// One thing a turn did to one file, with its dictionaries resolved.
#[derive(Debug, Serialize)]
pub struct Effect {
    pub at: i64,
    pub agent: String,
    pub did: reader::effects::Did,
    /// Absent when the subject was not named and nothing bounded it — the row
    /// still exists, and that is the point of it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// The command that did it, as the shell would have run it.
    pub command: String,
    pub reached: reader::shell::Reached,
    pub verdict: reader::doing::Verdict,
}

/// What one turn did, and how much of it there was.
#[derive(Debug, Serialize)]
pub struct Evidence {
    pub effects: Vec<Effect>,
    pub total: usize,
    /// How many of the total were subjects nobody could name.
    ///
    /// ⚠ **Reported beside the rows rather than left to be counted from them.**
    /// A page of two hundred effects that happens to contain no admission would
    /// otherwise read as a complete account of the turn.
    pub unnamed: usize,
}

/// GET /api/effects — what a turn actually did, and to what (owner only).
///
/// The question a reader asks standing on a `/api/doing` row: *which files, and
/// how do you know?* Keyed by `(agent, at)`, which that row already carries, so
/// this is a filter and not a join.
///
/// Owner-only for the reason the timeline is, and more so — a row carries the
/// command text verbatim.
pub async fn effects(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
    Query(query): Query<EffectsQuery>,
) -> Result<Json<Evidence>, AppError> {
    let log = app.effects();
    // A filter naming an agent the artefact has never seen matches nothing,
    // rather than matching everything — same rule as the timeline.
    let agent = match &query.agent {
        None => None,
        Some(want) => {
            let Some(at) = log
                .agents
                .iter()
                .position(|name| name.eq_ignore_ascii_case(want))
            else {
                return Ok(Json(Evidence {
                    effects: Vec::new(),
                    total: 0,
                    unnamed: 0,
                }));
            };
            Some(at as u32)
        }
    };
    let wanted = |row: &reader::effects::Row| {
        agent.is_none_or(|a| row.a == a)
            && query.at.is_none_or(|at| row.t == at)
            && query.path.as_ref().is_none_or(|want| {
                row.p
                    .and_then(|p| log.paths.get(p as usize))
                    .is_some_and(|path| path.contains(want.as_str()))
            })
    };
    let limit = query.limit.unwrap_or(PAGE).min(PAGE);
    let (mut total, mut unnamed, mut effects) = (0usize, 0usize, Vec::new());
    for row in log.rows.iter().rev().filter(|row| wanted(row)) {
        total += 1;
        unnamed += usize::from(row.k == reader::effects::Did::Unnamed);
        if effects.len() < limit {
            effects.push(Effect {
                at: row.t,
                agent: log.agents.get(row.a as usize).cloned().unwrap_or_default(),
                did: row.k,
                path: row.p.and_then(|p| log.paths.get(p as usize).cloned()),
                pattern: row.q.and_then(|q| log.patterns.get(q as usize).cloned()),
                host: row.h.and_then(|h| log.hosts.get(h as usize).cloned()),
                command: log
                    .commands
                    .get(row.c as usize)
                    .cloned()
                    .unwrap_or_default(),
                reached: row.r,
                verdict: row.v,
            });
        }
    }
    Ok(Json(Evidence {
        effects,
        total,
        unnamed,
    }))
}

fn share_json(app: &AppState) -> Value {
    match app.share.get() {
        Some(s) => {
            let url = app
                .cfg
                .public_base_url
                .as_deref()
                .map(|base| build_share_url(base, &s.token));
            json!({
                "active": true,
                "token": s.token,
                "url": url,
                "created_at": s.created_at,
                "last_accessed_at": s.last_accessed_at,
            })
        }
        None => json!({ "active": false }),
    }
}

/// GET /api/share — current share state (owner only).
pub async fn share_get(State(app): State<AppState>, OwnerOnly(_): OwnerOnly) -> Json<Value> {
    Json(share_json(&app))
}

/// POST /api/share — create or rotate the token.
pub async fn share_rotate(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
) -> Result<Json<Value>, AppError> {
    app.share.rotate()?;
    Ok(Json(share_json(&app)))
}

/// DELETE /api/share — revoke.
pub async fn share_revoke(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
) -> Result<Json<Value>, AppError> {
    app.share.revoke()?;
    Ok(Json(share_json(&app)))
}
