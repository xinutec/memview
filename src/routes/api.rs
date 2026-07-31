//! JSON API. All corpus reads admit the owner or a share-token holder;
//! share management is owner-only.

use std::collections::BTreeMap;

use axum::Json;
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
}

/// GET /api/memory/{name}
pub async fn memory(
    State(app): State<AppState>,
    ReadAccess(_): ReadAccess,
    Path(name): Path<String>,
) -> Result<Json<MemoryPage>, AppError> {
    let corpus = load_corpus(&app)?;
    let doc = corpus.get(&name).ok_or(AppError::NotFound)?;
    let (outlinks, dangling) = corpus.outlinks(doc);
    Ok(Json(MemoryPage {
        meta: doc.meta.clone(),
        html: render_markdown(&doc.body)?,
        backlinks: corpus.backlinks(&name),
        outlinks,
        dangling,
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

/// GET /api/search?q=
pub async fn search(
    State(app): State<AppState>,
    ReadAccess(_): ReadAccess,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, AppError> {
    let corpus = load_corpus(&app)?;
    Ok(Json(json!({ "hits": corpus.search(&query.q) })))
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

// -- history ---------------------------------------------------------------
//
// Owner-only, all of it. Not because isis is untrusted — it holds the corpus
// already and the fleet's threat model is losing data, not disclosing it — but
// because a share token is a deliberately public surface. Handing somebody a
// link to one memory must not also hand them every prompt ever typed.

/// What the history page needs to render before anyone searches: who the
/// sessions are and what they worked on. Deliberately WITHOUT the turns, which
/// are ~13 MB and are answered by the search endpoint instead.
#[derive(serde::Serialize)]
pub struct HistorySummary {
    pub generated: String,
    pub sessions: Vec<crate::history::Session>,
    pub projects: Vec<crate::history::Project>,
    pub turns: usize,
}

/// One search hit, with enough context to be worth reading in a list.
#[derive(serde::Serialize)]
pub struct HistoryHit {
    pub session: String,
    pub project: Option<String>,
    pub at: String,
    /// What was asked. Short — a prompt averages ninety characters.
    pub prompt: String,
    /// A window around the match in what Claude said, or empty when the match
    /// was in the prompt. Never the whole reply: those run to thousands of
    /// characters and a list of them is not a list of answers.
    pub reply: String,
    /// Which field matched, so the row can say why it is here.
    pub matched: &'static str,
    /// BM25 score, rounded. Exposed so a ranking regression is visible in the
    /// response rather than only in how the page happens to feel.
    pub score: f64,
}

/// Characters of context on each side of a match.
const SNIPPET_PAD: usize = 110;

/// A window around `pos` in `text`, on character boundaries.
fn snippet(text: &str, pos: usize, len: usize) -> String {
    let mut start = pos.saturating_sub(SNIPPET_PAD);
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (pos + len + SNIPPET_PAD).min(text.len());
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    let mut out = text[start..end].replace('\n', " ");
    if start > 0 {
        out.insert(0, '…');
    }
    if end < text.len() {
        out.push('…');
    }
    out
}

fn load_history(app: &AppState) -> Option<crate::history::History> {
    let path = app.cfg.history_file.as_ref()?;
    crate::history::History::load(std::path::Path::new(path))
}

/// GET /api/history — sessions and projects (owner only).
pub async fn history(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
) -> Result<Json<HistorySummary>, AppError> {
    let Some(h) = load_history(&app) else {
        // An empty summary rather than a 404: the page renders and says the
        // artefact has not been mined, which is a fact about this deployment
        // rather than an error in the request.
        return Ok(Json(HistorySummary {
            generated: String::new(),
            sessions: Vec::new(),
            projects: Vec::new(),
            turns: 0,
        }));
    };
    Ok(Json(HistorySummary {
        generated: h.generated,
        sessions: h.sessions,
        projects: h.projects,
        turns: h.turns.len(),
    }))
}

#[derive(serde::Deserialize)]
pub struct HistoryQuery {
    pub q: Option<String>,
    pub project: Option<String>,
    pub session: Option<String>,
}

#[derive(serde::Serialize, Default)]
pub struct HistorySearch {
    pub hits: Vec<HistoryHit>,
    /// Total matches, which may exceed `hits.len()`.
    pub total: usize,
    /// Every session the match set touches, most hits first — the answer to
    /// "which one said it" when the page cannot show them all.
    pub by_session: Vec<Tally>,
    pub by_project: Vec<Tally>,
}

/// How many turns come back. A search that answers with two thousand rows has
/// not answered anything; the totals below say what was left out.
const MAX_HITS: usize = 60;

/// Where the whole match set lives, regardless of what fitted on the page.
///
/// The page's question is often "WHICH session talked about this", and for a
/// common term no page of individual turns can answer it — "backup" matches 984
/// turns across nine sessions. A count per session answers it in one line.
#[derive(serde::Serialize)]
pub struct Tally {
    pub name: String,
    pub hits: usize,
}

/// GET /api/history/search — turns matching a query (owner only).
///
/// Every match is scored and the best are returned; the cap is applied AFTER
/// ranking. The first version applied it during the scan, so a common term came
/// back as an arbitrary slice of whichever sessions were read first — 984
/// matches for "backup" reduced to 100 rows from two sessions.
pub async fn history_search(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
    axum::extract::Query(query): axum::extract::Query<HistoryQuery>,
) -> Result<Json<HistorySearch>, AppError> {
    let Some(h) = load_history(&app) else {
        return Ok(Json(HistorySearch::default()));
    };
    let needle = query.q.unwrap_or_default();

    // Filters first: they define the candidate set the ranking scores against,
    // so a term's rarity is measured within the project being looked at rather
    // than across the whole corpus.
    let mut candidates: Vec<usize> = Vec::new();
    for (i, turn) in h.turns.iter().enumerate() {
        let project = turn
            .project
            .and_then(|p| h.projects.get(p))
            .map(|p| &p.name);
        if let Some(want) = &query.project
            && project != Some(want)
        {
            continue;
        }
        if let Some(want) = &query.session
            && h.sessions.get(turn.session).map(|s| &s.name) != Some(want)
        {
            continue;
        }
        if turn.prompt.is_empty() && turn.reply.is_empty() {
            continue;
        }
        candidates.push(i);
    }

    let name_of = |turn: &crate::history::Turn| {
        h.sessions
            .get(turn.session)
            .map(|s| s.name.clone())
            .unwrap_or_default()
    };
    let project_of = |turn: &crate::history::Turn| {
        turn.project
            .and_then(|p| h.projects.get(p))
            .map(|p| p.name.clone())
    };

    // An empty query lists the filtered set newest first — that is what makes
    // "show me everything heatcam" a query rather than a special case.
    let ordered: Vec<(usize, f64, bool)> = if needle.trim().is_empty() {
        let mut rows: Vec<usize> = candidates.clone();
        rows.sort_by(|a, b| h.turns[*b].at.cmp(&h.turns[*a].at));
        rows.into_iter().map(|i| (i, 0.0, false)).collect()
    } else {
        let docs: Vec<crate::rank::Doc<'_>> = candidates
            .iter()
            .map(|&i| crate::rank::Doc {
                prompt: &h.turns[i].prompt,
                reply: &h.turns[i].reply,
                at: &h.turns[i].at,
            })
            .collect();
        crate::rank::rank(&docs, &needle)
            .into_iter()
            .map(|s| (candidates[s.index], s.score, s.in_prompt))
            .collect()
    };

    let total = ordered.len();

    // Which sessions and projects the WHOLE match set lives in, before the page
    // cap. This is the part that answers "which one said it" for a broad term.
    let mut by_session: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_project: BTreeMap<String, usize> = BTreeMap::new();
    for (i, _, _) in &ordered {
        *by_session.entry(name_of(&h.turns[*i])).or_insert(0) += 1;
        if let Some(p) = project_of(&h.turns[*i]) {
            *by_project.entry(p).or_insert(0) += 1;
        }
    }
    let tally = |m: BTreeMap<String, usize>| {
        let mut v: Vec<Tally> = m
            .into_iter()
            .map(|(name, hits)| Tally { name, hits })
            .collect();
        v.sort_by(|a, b| b.hits.cmp(&a.hits).then(a.name.cmp(&b.name)));
        v
    };

    // Interleaved by session so one talkative session cannot fill the page.
    let picked = crate::rank::diversify(ordered, |(i, _, _)| name_of(&h.turns[*i]), MAX_HITS);

    let hits = picked
        .into_iter()
        .map(|(i, score, in_prompt)| {
            let turn = &h.turns[i];
            let (matched, reply) = if needle.trim().is_empty() {
                ("all", String::new())
            } else if in_prompt {
                ("prompt", String::new())
            } else {
                let at = turn
                    .reply
                    .to_lowercase()
                    .find(&needle.to_lowercase())
                    .or_else(|| {
                        // Fall back to the first term when the whole query is
                        // not literally present — the ranking matched on terms,
                        // so a snippet has to be found the same way.
                        crate::rank::tokenize(&needle)
                            .first()
                            .and_then(|t| turn.reply.to_lowercase().find(t.as_str()))
                    })
                    .unwrap_or(0);
                ("reply", snippet(&turn.reply, at, needle.len()))
            };
            HistoryHit {
                session: name_of(turn),
                project: project_of(turn),
                at: turn.at.clone(),
                prompt: turn.prompt.clone(),
                reply,
                matched,
                score: (score * 100.0).round() / 100.0,
            }
        })
        .collect();

    Ok(Json(HistorySearch {
        hits,
        total,
        by_session: tally(by_session),
        by_project: tally(by_project),
    }))
}
