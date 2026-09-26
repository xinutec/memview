//! JSON API. Every route admits the signed-in owner and nobody else.

use axum::Json;
use std::collections::BTreeMap;

use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::access::Owner;
use crate::error::AppError;
use crate::state::AppState;
use crate::store::{Corpus, Graph, MemoryMeta, render_markdown};

fn load_corpus(app: &AppState) -> Result<Corpus, AppError> {
    Ok(Corpus::load(&app.cfg.memory_dir)?)
}

/// GET /api/reading — the corpus survey. 404 rather than an empty body when
/// nothing has been mined, so the view can say "not mined yet".
pub async fn reading(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<reader::reading::CorpusRead>, AppError> {
    let summary = app.reading().ok_or(AppError::NotFound)?;
    Ok(Json((*summary).clone()))
}

/// Who is signed in, and whether signing in is on at all.
#[derive(Serialize)]
pub struct Me {
    user_id: String,
    display_name: String,
    auth_enabled: bool,
}

/// GET /api/me
pub async fn me(State(app): State<AppState>, Owner(user): Owner) -> Json<Me> {
    Json(Me {
        user_id: user.user_id,
        display_name: user.display_name,
        auth_enabled: app.cfg.auth.is_some(),
    })
}

/// GET /api/index — MEMORY.md rendered, links rewritten to /m/<name>.
pub async fn index(State(app): State<AppState>, Owner(_): Owner) -> Result<Json<Value>, AppError> {
    let corpus = load_corpus(&app)?;
    let md = corpus.index_md.ok_or(AppError::NotFound)?;
    // dev-lint: allow-wire-untyped pre-standard debt (DL-WIRE-UNTYPED-RESPONSE): give this handler a Serialize response struct when the route is next touched
    Ok(Json(json!({
        "html": render_markdown(&md)?,
        "count": corpus.docs.len(),
    })))
}

/// GET /api/memories — every memory's metadata.
pub async fn memories(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<Vec<MemoryMeta>>, AppError> {
    Ok(Json(load_corpus(&app)?.list()))
}

/// Who wrote a memory: the session its frontmatter names, and the agent that
/// session belongs to when the roster still knows. Optional because a memory can
/// outlive the transcript that wrote it — a handful predate the archive, not
/// pruning (memview#1240).
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
    /// Wikilink targets not written yet — surfaced, not hidden.
    dangling: Vec<String>,
    /// The session that wrote it, and the agent it was, where the mine knows.
    #[serde(skip_serializing_if = "Option::is_none")]
    origin: Option<Origin>,
}

/// The agent roster, or an empty one when nothing has been mined — normal on a
/// fresh checkout or CI.
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
    Owner(_): Owner,
    Path(name): Path<String>,
) -> Result<Json<MemoryPage>, AppError> {
    let corpus = load_corpus(&app)?;
    let doc = corpus.get(&name).ok_or(AppError::NotFound)?;
    let (outlinks, dangling) = corpus.outlinks(doc);
    let origin = doc.origin_session.as_ref().map(|session| Origin {
        agent: roster(&app).name_of_session(session).map(str::to_string),
        session: session.clone(),
    });
    Ok(Json(MemoryPage {
        meta: doc.meta.clone(),
        html: render_markdown(&doc.body)?,
        backlinks: corpus.backlinks(&name),
        outlinks,
        dangling,
        origin,
    }))
}

/// GET /api/graph — the corpus as a link graph, one payload because a layout
/// needs every node before it can place any. Every node carries its
/// `description`, so measure the size rather than trusting a number here —
/// the last one rotted by two orders of magnitude. See memview#1306.
pub async fn graph(State(app): State<AppState>, Owner(_): Owner) -> Result<Json<Graph>, AppError> {
    let mut graph = load_corpus(&app)?.graph();
    if let Some(path) = &app.cfg.couse_file
        && let Some(couse) = crate::couse::CoUse::load(std::path::Path::new(path))
    {
        graph.as_of = Some(couse.generated);
        graph.usage = couse.usage;
        graph.affinities = couse.pairs;
    }
    Ok(Json(graph))
}

#[derive(Deserialize)]
pub struct SearchQuery {
    q: String,
}

/// The mined per-memory usage, or an empty map — normal on a machine with no
/// transcripts; only the ordering among comparable answers changes.
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
    Owner(_): Owner,
    Query(query): Query<SearchQuery>,
) -> Result<Json<crate::store::SearchResult>, AppError> {
    let corpus = load_corpus(&app)?;
    Ok(Json(corpus.search(&query.q, &usage_of(&app))))
}

/// GET /api/agents — which named session works where.
pub async fn agents(
    State(app): State<AppState>,
    Owner(_): Owner,
) -> Result<Json<crate::agents::Agents>, AppError> {
    Ok(Json(roster(&app)))
}

/// GET /api/work?q= — who has been changing the files a query names. The
/// companion to `/api/search`: what was WORKED ON rather than written down.
pub async fn work(
    State(app): State<AppState>,
    Owner(_): Owner,
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
    /// How many effects `/api/effects` would return for this row. On the row because
    /// 12.6% of rows have NONE, and a tap to learn "nothing was recorded" is a tap
    /// wasted; the median carries 10 and the 90th percentile 85.
    pub effects: u32,
    /// Which instruction this was part of — an index into [`Timeline::episodes`]. An
    /// id, not the episode: a stretch of one instruction is dozens of rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub episode: Option<usize>,
}

/// One instruction, as much of it as the page can see. Not `Episode`, which
/// `reader::doing::Episode` already is — the wire-mirror check pairs types BY NAME.
#[derive(Debug, Serialize)]
pub struct TimelineEpisode {
    pub agent: String,
    /// The minute of its first row, which may be older than anything on this page.
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
    /// Kinds of work in the filtered range, biggest first — the shape of the answer.
    pub summary: Vec<(String, usize)>,
    pub total: usize,
    pub failed: usize,
}

/// How many moments one request may take: the artefact is two hundred thousand rows.
const PAGE: usize = 200;

/// GET /api/doing — what the sessions did, newest first. Derived throughout; no
/// command line or prompt text exists in the artefact.
pub async fn doing(
    State(app): State<AppState>,
    Owner(_): Owner,
    Query(query): Query<DoingQuery>,
) -> Result<Json<Timeline>, AppError> {
    let log = app.doing();
    // Counted with the artefact rather than here — see `effects_and_counts`.
    let (effects_log, counts) = app.effects_and_counts();
    // The two artefacts index their agents separately, so the join is by name —
    // built ONCE, not per row.
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
    // A filter naming something the corpus has never seen matches nothing, not everything.
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
    // The episodes this page touches, numbered for the page.
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
    /// Whether this use may be attributed — [`reader::doing::Verdict::admits`].
    /// Computed here because a client joining the two fields would be a third copy
    /// of the rule (`console/src/parse.rs` is the second); the timeline once decided
    /// from `reached` alone, typed `boolean` against a wire sending `"a"`
    /// (memview#1459). One-sided: `false` means "cannot say".
    pub certain: bool,
}

/// What one turn did, and how much of it there was.
#[derive(Debug, Serialize)]
pub struct Evidence {
    pub effects: Vec<Effect>,
    pub total: usize,
    /// How many of the total were subjects nobody could name. Reported beside the
    /// rows: a page that happens to contain no admission would read as complete.
    pub unnamed: usize,
}

/// GET /api/effects — what a turn actually did, and to what. Keyed by
/// `(agent, at)`, which a `/api/doing` row already carries.
pub async fn effects(
    State(app): State<AppState>,
    Owner(_): Owner,
    Query(query): Query<EffectsQuery>,
) -> Result<Json<Evidence>, AppError> {
    let log = app.effects();
    // An unknown agent matches nothing — same rule as the timeline.
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
        unnamed += usize::from(matches!(
            row.k,
            reader::effects::Did::Unnamed | reader::effects::Did::Located
        ));
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
                certain: row.v.admits(row.r),
            });
        }
    }
    Ok(Json(Evidence {
        effects,
        total,
        unnamed,
    }))
}
