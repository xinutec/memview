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
