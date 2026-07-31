//! JSON API. All corpus reads admit the owner or a share-token holder;
//! share management is owner-only.

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

/// How many hits to return. A search that answers with two thousand rows has
/// not answered anything, and the count is reported separately so a truncated
/// result never reads as a complete one.
const MAX_HITS: usize = 100;

#[derive(serde::Deserialize)]
pub struct HistoryQuery {
    pub q: Option<String>,
    pub project: Option<String>,
    pub session: Option<String>,
}

#[derive(serde::Serialize)]
pub struct HistorySearch {
    pub hits: Vec<HistoryHit>,
    /// Total matches, which may exceed `hits.len()`.
    pub total: usize,
}

/// GET /api/history/search — turns matching a query (owner only).
///
/// Searched on the server rather than shipped to the client: the turn list is
/// ~13 MB, and a plain substring scan over it in Rust costs single-digit
/// milliseconds, so sending it to a phone over the VPN would be the slow half
/// of an otherwise instant answer.
pub async fn history_search(
    State(app): State<AppState>,
    OwnerOnly(_): OwnerOnly,
    axum::extract::Query(query): axum::extract::Query<HistoryQuery>,
) -> Result<Json<HistorySearch>, AppError> {
    let Some(h) = load_history(&app) else {
        return Ok(Json(HistorySearch {
            hits: Vec::new(),
            total: 0,
        }));
    };
    let needle = query.q.unwrap_or_default().to_lowercase();
    let mut hits = Vec::new();
    let mut total = 0;
    for turn in &h.turns {
        let project = turn
            .project
            .and_then(|i| h.projects.get(i))
            .map(|p| p.name.clone());
        if let Some(want) = &query.project
            && project.as_deref() != Some(want.as_str())
        {
            continue;
        }
        let session = h
            .sessions
            .get(turn.session)
            .map(|s| s.name.clone())
            .unwrap_or_default();
        if let Some(want) = &query.session
            && session != *want
        {
            continue;
        }
        // Both sides are searched. Asking "what did I say" is the easy recall
        // problem; "I remember it SAYING something, but not which session" is
        // the common one, and prompts alone cannot answer it.
        let mut reply = String::new();
        let matched = if needle.is_empty() {
            // An empty query lists the filtered set, which is what makes "show
            // me everything heatcam" a query rather than a special case.
            "all"
        } else if turn.prompt.to_lowercase().contains(&needle) {
            "prompt"
        } else if let Some(at) = turn.reply.to_lowercase().find(&needle) {
            reply = snippet(&turn.reply, at, needle.len());
            "reply"
        } else {
            continue;
        };
        total += 1;
        if hits.len() < MAX_HITS {
            hits.push(HistoryHit {
                session,
                project,
                at: turn.at.clone(),
                prompt: turn.prompt.clone(),
                reply,
                matched,
            });
        }
    }
    // Newest first: looking for something you did recently is the common case.
    hits.sort_by(|a, b| b.at.cmp(&a.at));
    Ok(Json(HistorySearch { hits, total }))
}
