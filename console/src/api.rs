//! The JSON API, and the event stream behind it.
//!
//! A session is read as server-sent events: the transcript so far, then live.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};

use crate::protocol as console_protocol;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::sse::{Event as Sse, KeepAlive};
use axum::response::{IntoResponse, Response, Sse as SseResponse};
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};
use tower_http::compression::CompressionLayer;
use tower_http::compression::predicate::SizeAbove;

use crate::protocol::Event;
use crate::roster::Roster;
use crate::session::{Stamped, Summary};
use crate::trace;

// dev-lint: allow-unauthenticated-route served only behind a client certificate or on loopback (main.rs)
pub fn router(roster: Arc<Roster>) -> Router {
    Router::new()
        .route("/api/state", get(state))
        .route("/api/sessions", post(start))
        .route("/api/past", get(past))
        .route("/api/sessions/{id}/input", post(input))
        .route("/api/sync/drafts", get(pull_drafts).post(push_drafts))
        // Its own body limit, so [`crate::images::keep`] can say why an image is refused.
        .route(
            "/api/sessions/{id}/image",
            post(show).layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route("/api/sessions/{id}/images/{name}", get(picture))
        .route("/api/picture", get(elsewhere))
        .route("/api/sessions/{id}/unhold", post(unhold))
        .route("/api/sessions/{id}/decide", post(decide))
        .route("/api/sessions/{id}/mode", post(mode))
        .route("/api/sessions/{id}/rename", post(rename))
        .route("/api/sessions/{id}/stop", post(stop))
        .route("/api/sessions/{id}/revive", post(revive))
        .route("/api/sessions/{id}", delete(forget))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/earlier", get(earlier))
        .route("/api/sessions/{id}/landmarks", get(landmarks))
        .route("/api/sessions/{id}/parse", post(parse))
        .route("/api/sessions/{id}/edits", get(edits))
        .route("/api/sessions/{id}/workflows/{run}", get(workflow))
        .route(
            "/api/sessions/{id}/workflows/{run}/agents/{agent}",
            get(workflow_agent),
        )
        .route("/api/hook", post(hook))
        .route("/api/sudo", post(sudo))
        .route("/api/login", post(login))
        .route("/api/login/code", post(login_code))
        .route("/api/sessions/{id}/sudo", post(sudo_answer))
        .route("/api/sessions/{id}/tasks", get(tasks))
        .route("/api/sessions/{id}/tasks/{task}", get(task))
        .route("/api/reading", get(reading))
        .route("/api/telemetry", post(trace::record))
        // The stream too: tower-http flushes when the body pauses, so live events are
        // not held back. Here, not in `main`, so the tests run this stack.
        .layer(CompressionLayer::new().compress_when(SizeAbove::new(SMALL)))
        .with_state(roster)
}

/// GET /api/reading — the corpus survey the nightly mined.
async fn reading() -> Result<Json<reader::reading::CorpusRead>, StatusCode> {
    let path = std::env::var("READING_FILE").unwrap_or(
        reader::home::cache("reading.json")
            .to_string_lossy()
            .into_owned(),
    );
    // Not mined yet is a 404, never a survey of zeroes.
    let text = std::fs::read_to_string(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    serde_json::from_str(&text)
        .map(Json)
        .map_err(|_| StatusCode::NOT_FOUND)
}

/// Below this many bytes a response goes uncompressed.
const SMALL: u64 = 1024;

/// The most pictures one message carries: a few screenshots, on one row of a phone.
pub const PICTURES: usize = 5;

/// How large a request carrying pictures may be: [`PICTURES`] of
/// [`crate::images::LIMIT`] as base64, with room to spare.
const BODY_LIMIT: usize = crate::images::LIMIT * 2 * PICTURES;

/// Everything a client needs to draw the front page in one request.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Overview {
    /// Where a session may be started — these and anything inside them.
    pub dirs: Vec<String>,
    /// The repositories inside those, for the client's picker.
    pub repos: Vec<String>,
    pub sessions: Vec<Summary>,
    /// A fingerprint of the bundle being served; the client reloads when it changes.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub bundle: Option<String>,
    /// How much of the subscription is spent, once any reading has arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub usage: Option<crate::usage::Reading>,
    /// What each conversation is about, by session id, written by a model.
    pub gists: std::collections::BTreeMap<String, crate::gist::Gist>,
    /// Who holds which tasks — see [`crate::tasks`].
    pub tasks: crate::tasks::Sweep,
    /// Each conversation's unsent words, by session id.
    pub drafts: std::collections::BTreeMap<String, crate::drafts::Draft>,
    /// The memory index's size against where Claude Code cuts it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub index: Option<IndexSize>,
}

/// `MEMORY.md`'s size and its ceiling, in bytes.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct IndexSize {
    pub bytes: u64,
    pub ceiling: u64,
}

/// Read per request: a stat, and the file changes whenever a session edits it.
fn index_size() -> Option<IndexSize> {
    let path = reader::home::memory_dir().join("MEMORY.md");
    Some(IndexSize {
        bytes: std::fs::metadata(path).ok()?.len(),
        ceiling: reader::home::INDEX_CEILING as u64,
    })
}

/// The bundle's identity: a hash of index.html, which names the hashed chunks.
fn bundle(dir: Option<&str>) -> Option<String> {
    use sha2::{Digest, Sha256};
    let page = std::fs::read(format!("{}/index.html", dir?)).ok()?;
    Some(hex::encode(Sha256::digest(&page))[..16].to_string())
}

async fn state(State(roster): State<Arc<Roster>>) -> Json<Overview> {
    Json(Overview {
        bundle: bundle(roster.config().static_dir.as_deref()),
        // From memory, never the network.
        usage: roster.usage().reading(&roster.spent()).await,
        dirs: roster
            .config()
            .dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect(),
        repos: roster.config().repos(),
        sessions: roster.list(),
        // From memory; written on its own timer.
        gists: roster.gists(),
        drafts: roster.drafts().all(),
        // Off the executor — see [`Roster::tasks`].
        tasks: roster.tasks().await,
        index: index_size(),
    })
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Start {
    pub dir: String,
    /// The first instruction. Optional: a session can be opened and then talked to.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub prompt: Option<String>,
    /// A conversation to pick up rather than starting a new one. Its id is kept.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub resume: Option<String>,
}

async fn start(
    State(roster): State<Arc<Roster>>,
    Json(body): Json<Start>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = match body.resume.as_deref().filter(|id| !id.trim().is_empty()) {
        Some(id) => roster.resume(&body.dir, id),
        None => roster.start(&body.dir),
    }
    .map_err(|err| (StatusCode::BAD_REQUEST, err))?;
    if let Some(prompt) = body.prompt.as_deref().filter(|p| !p.trim().is_empty()) {
        session.send(prompt).await.map_err(|err| {
            // The session exists but did not take the message: say both.
            (
                StatusCode::BAD_GATEWAY,
                format!("started {} but could not send: {err:#}", session.id),
            )
        })?;
    }
    Ok(Json(roster.summary_of(&session)))
}

#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Message {
    pub text: String,
}

async fn input(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<Message>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    // A receipt — a doubled message leaves no other trace. The length, not the words.
    tracing::info!(
        "{id}: accepted {} characters to send",
        body.text.chars().count()
    );
    session
        .send(&body.text)
        .await
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(roster.summary_of(&session)))
}

/// How far a client has already pulled.
#[derive(Debug, Deserialize)]
pub struct Since {
    /// Absent on a first pull, which then asks for everything.
    #[serde(default)]
    pub since: Option<u64>,
}

/// Every draft past the caller's checkpoint, by the life app's sync protocol.
async fn pull_drafts(
    State(roster): State<Arc<Roster>>,
    Query(from): Query<Since>,
) -> Json<crate::drafts::PullResponse> {
    Json(roster.drafts().pull(from.since.unwrap_or(0)))
}

/// Apply a batch of edits and answer with the ones that lost — a 200, since one
/// batch can both land and lose. The only way drafts are written.
async fn push_drafts(
    State(roster): State<Arc<Roster>>,
    Json(entries): Json<Vec<crate::drafts::PushEntry>>,
) -> Json<Vec<crate::drafts::DraftDoc>> {
    Json(roster.drafts().push(entries))
}

/// Pictures from the phone, with whatever is being said about them.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Shown {
    /// In the order they are shown; at least one, at most [`PICTURES`].
    pub pictures: Vec<ShownPicture>,
    /// What was said about them. Optional: a screenshot alone is a complete message.
    #[serde(default)]
    pub text: String,
}

/// One picture in a [`Shown`].
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct ShownPicture {
    /// The bytes, base64.
    pub data: String,
    /// What the client believes it is sending, checked against the bytes.
    #[serde(default)]
    pub media_type: String,
}

/// Hand back a picture sent to a session, cached hard: a kept picture never
/// changes. [`crate::images::find`] reads only names it could have written.
async fn picture(Path((id, name)): Path<(String, String)>) -> Response {
    match crate::images::find(&crate::images::images_root(), &id, &name) {
        Some((bytes, media_type)) => (
            [
                (header::CONTENT_TYPE, media_type),
                (
                    header::CACHE_CONTROL,
                    "private, max-age=31536000, immutable",
                ),
            ],
            bytes,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "no such picture").into_response(),
    }
}

/// GET /api/picture?url=… — a picture a session pointed at, fetched from here. Not
/// cached: the file is re-rendered. See [`crate::images::fetch`].
async fn elsewhere(Query(asked): Query<Elsewhere>) -> Response {
    let fetched = crate::images::fetch(&asked.url).await;
    // Decoding and scaling is CPU work, kept off the executor.
    let fetched = match fetched {
        Ok(got) if asked.small => tokio::task::spawn_blocking(move || {
            crate::images::thumbnail(got, crate::images::THUMBNAIL)
        })
        .await
        .map_err(|why| crate::images::Reason::Answered(format!("could not scale it: {why}"))),
        other => other,
    };
    match fetched {
        Ok(got) => (
            [
                (header::CONTENT_TYPE, got.media_type),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            got.bytes,
        )
            .into_response(),
        // Whose fault, in the status as well as the words.
        Err(why @ crate::images::Reason::Asked(_)) => {
            (StatusCode::BAD_REQUEST, why.to_string()).into_response()
        }
        Err(why @ crate::images::Reason::Answered(_)) => {
            (StatusCode::BAD_GATEWAY, why.to_string()).into_response()
        }
    }
}

/// The URL to fetch, as it came off the query string, and whether to scale it down
/// to a thumbnail.
#[derive(Deserialize)]
struct Elsewhere {
    url: String,
    #[serde(default)]
    small: bool,
}

/// Show a session pictures — its own route: megabytes, files, their own failures.
/// Every picture is kept before any is sent, so a refusal sends nothing.
async fn show(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<Shown>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    use base64::Engine as _;

    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    if body.pictures.is_empty() || body.pictures.len() > PICTURES {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{} pictures, and a message carries 1 to {PICTURES}",
                body.pictures.len()
            ),
        ));
    }
    // UTC: `now_local` is refused in a threaded program here.
    let stamp = time::OffsetDateTime::now_utc()
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]-[hour][minute][second]Z"
        ))
        .unwrap_or_else(|_| "image".to_string());
    let mut kept = Vec::with_capacity(body.pictures.len());
    for picture in &body.pictures {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(picture.data.trim())
            .map_err(|why| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("that image is not base64: {why}"),
                )
            })?;
        let held = crate::images::keep(
            &crate::images::images_root(),
            &id,
            &picture.media_type,
            &bytes,
            &stamp,
        )
        .map_err(|why| (StatusCode::BAD_REQUEST, why))?;
        tracing::info!(
            "showing {id} a {} of {} bytes, kept at {}",
            held.media_type,
            bytes.len(),
            held.path.display()
        );
        kept.push(held);
    }
    let pictures = body
        .pictures
        .iter()
        .zip(&kept)
        .map(|(picture, held)| crate::protocol::Picture {
            media_type: &held.media_type,
            base64: picture.data.trim(),
            kept: &held.path,
        })
        .collect::<Vec<_>>();
    session
        .show(&body.text, &pictures)
        .await
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(roster.summary_of(&session)))
}

/// What to do about one question.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Decision {
    /// The control-request id from the `ask` event.
    pub id: String,
    pub allow: bool,
    /// Why not. Ignored on an allow; the session is told it on a refusal.
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub why: Option<String>,
    /// What was said about a question; refused for any other tool.
    #[serde(default, flatten)]
    pub reply: console_protocol::Reply,
}

/// The refusal a client sends when it does not say why.
const REFUSED: &str = "Refused from the console.";

async fn decide(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<Decision>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    session
        .decide(
            &body.id,
            body.allow,
            body.why.as_deref().unwrap_or(REFUSED),
            // An ordinary approval says nothing.
            Some(&body.reply).filter(|reply| !reply.is_empty()),
        )
        .await
        // Usually answered a moment ago on another screen.
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(roster.summary_of(&session)))
}

/// What a client asks for when changing a session's permission mode.
#[derive(serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
struct ModeChange {
    mode: crate::modes::Mode,
}

async fn mode(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<ModeChange>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    // An unknown mode is refused here, not by the session out of sight.
    if let crate::modes::Mode::Unknown(name) = &body.mode {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("{name} is not a permission mode"),
        ));
    }
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    session
        .set_mode(&body.mode)
        .await
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    // Only once taken, so a failed mode does not return at the next resume.
    roster.remember_mode(&id, &body.mode);
    Ok(Json(roster.summary_of(&session)))
}

/// What to call a conversation. See [`Session::rename`].
#[derive(serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
struct Renaming {
    title: String,
}

/// Rename a conversation, even a working one. The summary keeps the old name until
/// the CLI's `custom-title` line is read.
async fn rename(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<Renaming>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "a name cannot be blank".into()));
    }
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    session
        .rename(title)
        .await
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(roster.summary_of(&session)))
}

/// Take back a command held for the turn's end; one already released is not an error.
async fn unhold(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<Message>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    let dropped = session.forget_held(&body.text);
    tracing::info!(
        "{id}: {} a held command",
        if dropped {
            "took back"
        } else {
            "was not holding"
        }
    );
    Ok(Json(roster.summary_of(&session)))
}

async fn stop(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    session.stop().await;
    Ok(Json(roster.summary_of(&session)))
}

/// Restart a session that stopped listening — [`Roster::revive`]. Slow: it waits
/// for the old process to go, so one transcript never has two writers.
async fn revive(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .revive(&id)
        .await
        .map_err(|why| (StatusCode::CONFLICT, why))?;
    Ok(Json(roster.summary_of(&session)))
}

async fn forget(State(roster): State<Arc<Roster>>, Path(id): Path<String>) -> impl IntoResponse {
    if roster.forget(&id) {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::NOT_FOUND
    }
}

/// What was said before the page the reader already has: the next page back,
/// read by the same reader as the seed.
#[derive(serde::Deserialize)]
struct Earlier {
    /// The byte offset the reader's page began at; absent for the newest page.
    #[serde(default)]
    before: Option<u64>,
}

#[derive(serde::Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
struct Page {
    events: Vec<console_protocol::Timed>,
    /// The cursor for the page before this one. Zero means the start.
    from: u64,
}

/// Everywhere in this conversation worth jumping to.
/// The first ask parses the whole transcript, off the executor; [`crate::marks`]
/// keeps the result.
async fn landmarks(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::past::Landmark>>, (StatusCode, String)> {
    // Through the roster: only a transcript this console owns.
    roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    let root = crate::past::projects_root();
    let path = crate::past::transcript_of(&root, &id).ok_or((
        StatusCode::NOT_FOUND,
        format!("no transcript on disk for {id}"),
    ))?;

    let began = std::time::Instant::now();
    let marks = roster.marks();
    let held = id.clone();
    let found = tokio::task::spawn_blocking(move || marks.of(&held, &path))
        .await
        .map_err(|err| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not read the transcript: {err}"),
            )
        })?;
    tracing::info!("{id}: {} landmark(s) in {:?}", found.len(), began.elapsed());
    Ok(Json(found))
}

async fn earlier(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Query(asked): Query<Earlier>,
) -> Result<Json<Page>, (StatusCode, String)> {
    // Through the roster, as above.
    roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    let root = crate::past::projects_root();
    let path = crate::past::transcript_of(&root, &id).ok_or((
        StatusCode::NOT_FOUND,
        format!("no transcript on disk for {id}"),
    ))?;
    let page = crate::past::page(&path, asked.before);
    tracing::info!(
        "{id}: {} earlier events before byte {:?}, page starts at {}",
        page.events.len(),
        asked.before,
        page.from
    );
    Ok(Json(Page {
        events: page.events,
        from: page.from,
    }))
}

/// One event on the wire, numbered, so a reconnect's `Last-Event-ID` fills the gap.
fn wire(stamped: Stamped) -> Sse {
    Sse::default()
        .id(stamped.seq.to_string())
        .json_data(console_protocol::Timed {
            at: stamped.at,
            event: stamped.event,
        })
        .unwrap_or_else(|err| {
            Sse::default().data(format!("{{\"kind\":\"trouble\",\"detail\":\"{err}\"}}"))
        })
}

/// Where a client says it had got to, when it is asking rather than reconnecting.
#[derive(Debug, Deserialize)]
struct Resume {
    /// The last sequence number held, for a page that reopens its stream. A string,
    /// so an unreadable value is ignored rather than a 400.
    #[serde(default)]
    after: Option<String>,
}

/// Where a client holds the transcript through: the header, else `?after=`.
pub fn resume_from(headers: &HeaderMap, asked: Option<&str>) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| asked.and_then(|value| value.parse::<u64>().ok()))
}

/// What a client holding nothing is sent: the transcript's last page, read before
/// the number is taken so nothing arrives twice, then any open question. Which
/// console-only events are replayed is decided in `tests/provenance.rs`.
fn cold(id: &str, session: &crate::session::Session) -> Option<(Vec<Sse>, u64)> {
    let root = crate::past::projects_root();
    let path = crate::past::transcript_of(&root, id)?;
    let page = crate::past::page(&path, None);
    if page.events.is_empty() {
        return None;
    }
    let through = session.issued();
    let earlier = page.events.len();
    // The page's own time, not the connection's.
    let ends = page.events.last().and_then(|timed| timed.at);
    // Unnumbered until the closing `joined`, so a half-sent seed is asked for again.
    let mut held: Vec<Sse> = page
        .events
        .into_iter()
        .map(|timed| unnumbered(timed.at, timed.event))
        .collect();
    held.push(wire(Stamped {
        seq: through,
        at: ends,
        event: Event::Joined {
            earlier,
            from: page.from,
            // A reader joined; nothing restarted.
            restarted: false,
        },
    }));
    // Open questions, which no transcript holds, or a blocked session shows nothing
    // to answer. Unnumbered: the real `Ask` is at or below `through`.
    for (id, question) in session.asking() {
        held.push(unnumbered(ends, question.ask(id)));
    }
    Some((held, through))
}

/// One event with no `id:`, so the browser keeps quoting the last number it can
/// vouch for — see [`cold`].
fn unnumbered(at: Option<i64>, event: Event) -> Sse {
    Sse::default()
        .json_data(console_protocol::Timed { at, event })
        .unwrap_or_else(|err| {
            Sse::default().data(format!("{{\"kind\":\"trouble\",\"detail\":\"{err}\"}}"))
        })
}

async fn events(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Query(asked): Query<Resume>,
    headers: HeaderMap,
) -> Result<SseResponse<impl Stream<Item = Result<Sse, Infallible>>>, (StatusCode, String)> {
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;

    let after = resume_from(&headers, asked.after.as_deref());

    // Subscribe first, so an event between the two is late rather than lost;
    // `through` drops its second copy.
    let live = BroadcastStream::new(session.listen());
    let backlog = session.since(after);
    // A client holding nothing is seeded from the transcript, else the log.
    let (held, through) = match backlog.resumed {
        true => (
            backlog.events.into_iter().map(wire).collect(),
            backlog.through,
        ),
        false => cold(&id, &session).unwrap_or_else(|| {
            (
                backlog.events.into_iter().map(wire).collect(),
                backlog.through,
            )
        }),
    };
    tracing::info!(
        "{id}: {} events for a reader at {after:?} ({}), holding through {through}",
        held.len(),
        if backlog.resumed {
            "resumed"
        } else {
            "from the top"
        }
    );

    // A named event about the connection: the page empties what it holds.
    let prelude = tokio_stream::iter((!backlog.resumed).then(|| {
        Sse::default()
            .event("reset")
            .data("this stream starts again from the beginning")
    }));

    let held = tokio_stream::iter(held);

    // Where replay ends on this connection, so a replayed `turn` does not read as
    // idle. Per connection, unlike `Joined`, which the log can trim.
    let caught_up = tokio_stream::iter([Sse::default()
        .event("caught-up")
        .data("everything after this is happening now")]);

    let live = live.filter_map(move |got| match got {
        // Already sent as part of the backlog.
        Ok(stamped) if stamped.seq <= through => None,
        Ok(stamped) => Some(wire(stamped)),
        // The channel dropped events; unnumbered, so the client keeps its last good id.
        Err(_) => Some(
            Sse::default()
                .json_data(Event::Trouble {
                    detail: "the console dropped events for this client".to_string(),
                })
                .unwrap_or_else(|err| {
                    Sse::default().data(format!("{{\"kind\":\"trouble\",\"detail\":\"{err}\"}}"))
                }),
        ),
    });

    let stream = prelude
        .chain(held)
        .chain(caught_up)
        .chain(live)
        .map(Ok::<Sse, Infallible>);

    Ok(SseResponse::new(stream).keep_alive(
        // Keeps a silent connection from being closed along the way.
        KeepAlive::new().interval(Duration::from_secs(15)),
    ))
}

/// Conversations that could be picked up, within the configured directories.
async fn past(State(roster): State<Arc<Roster>>) -> Json<Vec<crate::past::Conversation>> {
    let root = crate::past::projects_root();
    let allowed: Vec<crate::past::Conversation> = crate::past::conversations(&root)
        .into_iter()
        .filter(|conversation| roster.config().resolve(&conversation.dir).is_ok())
        .collect();
    Json(allowed)
}

/// One `Bash` command, read the way the index reads it. See [`crate::parse`].
/// The directory comes from the session, never the body, so no caller picks it.
async fn parse(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(asked): Json<crate::parse::Asked>,
) -> Json<crate::parse::Parsed> {
    let dir = match roster.get(&id) {
        Some(session) => Some(session.dir.to_string_lossy().into_owned()),
        None => crate::past::dir_of(&crate::past::projects_root(), &id),
    };
    let home = std::env::var("HOME").unwrap_or_default();
    Json(crate::parse::parsed(&asked, dir.as_deref(), &home))
}

/// A session's task list, without the prose — ended sessions included.
async fn tasks(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Json<Vec<crate::tasks::Task>> {
    Json(roster.task_list(&id).await)
}

/// What one task says, fetched when it is opened rather than with the list.
async fn task(
    State(roster): State<Arc<Roster>>,
    // The session in the path is unused: task numbers are unique.
    Path((_session, task)): Path<(String, String)>,
) -> impl IntoResponse {
    match roster.task_detail(&task).await {
        Some(description) => Json(Described { description }).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
struct Described {
    description: String,
}

/// The app for its own routes; a 404 for a missing file — a path whose last
/// segment has a dot — rather than the index served as a font.
pub fn spa(index: &str, path: &str) -> axum::response::Response {
    use axum::response::IntoResponse as _;

    if path
        .rsplit('/')
        .next()
        .is_some_and(|last| last.contains('.'))
    {
        return (axum::http::StatusCode::NOT_FOUND, "not found").into_response();
    }
    match std::fs::read_to_string(index) {
        Ok(page) => axum::response::Html(page).into_response(),
        Err(error) => {
            tracing::error!("the app's index could not be read: {error}");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "no index").into_response()
        }
    }
}

/// GET /api/sessions/{id}/edits — predicted changes, and which diverged, from disk.
async fn edits(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Json<crate::edits::Record> {
    Json(roster.edits().of(&id))
}

/// The directory of one workflow run launched by a session this console owns.
fn run_dir(
    roster: &Roster,
    id: &str,
    run: &str,
) -> Result<std::path::PathBuf, (StatusCode, String)> {
    roster
        .get(id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    if !crate::workflows::is_run(run) {
        return Err((StatusCode::BAD_REQUEST, format!("not a run id: {run}")));
    }
    let transcript = crate::past::transcript_of(&crate::past::projects_root(), id).ok_or((
        StatusCode::NOT_FOUND,
        format!("no transcript on disk for {id}"),
    ))?;
    Ok(crate::workflows::dir(&transcript, run))
}

/// GET /api/sessions/{id}/workflows/{run} — the run's agents by phase, and what
/// each working agent last called.
async fn workflow(
    State(roster): State<Arc<Roster>>,
    Path((id, run)): Path<(String, String)>,
) -> Result<Json<crate::workflows::Run>, (StatusCode, String)> {
    let dir = run_dir(&roster, &id, &run)?;
    tokio::task::spawn_blocking(move || crate::workflows::run(&dir))
        .await
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, format!("{err:#}")))?
        .map(Json)
        .ok_or((StatusCode::NOT_FOUND, format!("no journal for {run}")))
}

/// Where in an agent's transcript to read: the page before `before`, what came
/// after `after`, or with neither the newest page.
#[derive(Debug, Deserialize)]
struct Stretch {
    before: Option<u64>,
    after: Option<u64>,
}

/// A stretch of one agent's transcript, and the cursors on either side of it.
#[derive(Debug, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
struct Stretched {
    events: Vec<console_protocol::Timed>,
    /// The cursor for the page before this stretch. Zero means the start.
    from: u64,
    /// The cursor to ask `after` for what comes next.
    to: u64,
}

/// GET /api/sessions/{id}/workflows/{run}/agents/{agent}
async fn workflow_agent(
    State(roster): State<Arc<Roster>>,
    Path((id, run, agent)): Path<(String, String, String)>,
    Query(asked): Query<Stretch>,
) -> Result<Json<Stretched>, (StatusCode, String)> {
    let dir = run_dir(&roster, &id, &run)?;
    if !crate::workflows::is_agent(&agent) {
        return Err((StatusCode::BAD_REQUEST, format!("not an agent id: {agent}")));
    }
    let path = crate::workflows::transcript(&dir, &agent);
    if !path.is_file() {
        return Err((StatusCode::NOT_FOUND, format!("no transcript for {agent}")));
    }
    let read = move || match (asked.before, asked.after) {
        (_, Some(after)) => {
            let (events, to) = crate::past::since(&path, after);
            Stretched {
                events,
                from: after,
                to,
            }
        }
        (before, None) => {
            let to = before.unwrap_or_else(|| crate::past::written(&path));
            let page = crate::past::page(&path, Some(to));
            Stretched {
                events: page.events,
                from: page.from,
                to,
            }
        }
    };
    tokio::task::spawn_blocking(read)
        .await
        .map(Json)
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

/// What a hook sends, every field defaulted: an unreadable payload must not block a call.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Hooked {
    hook_event_name: Hook,
    session_id: String,
    tool_name: String,
    tool_use_id: String,
    cwd: String,
    tool_input: serde_json::Value,
    tool_response: serde_json::Value,
}

/// When in a call's life a hook fired. Only the two this answers are named.
#[derive(Deserialize, Default, PartialEq)]
enum Hook {
    PreToolUse,
    PostToolUse,
    #[default]
    #[serde(other)]
    Other,
}

/// POST /api/hook — before a `Bash` call, predict its changes; after, report any
/// divergence. The hook waits on this.
async fn hook(State(roster): State<Arc<Roster>>, Json(hooked): Json<Hooked>) -> StatusCode {
    let crate::call::Call::Bash { command, .. } =
        crate::call::Call::read(&hooked.tool_name, &hooked.tool_input)
    else {
        return StatusCode::NO_CONTENT;
    };
    let edits = roster.edits();
    let call = hooked.tool_use_id.clone();
    match hooked.hook_event_name {
        Hook::PreToolUse => {
            let (session, cwd) = (hooked.session_id.clone(), hooked.cwd.clone());
            let predicted =
                tokio::task::spawn_blocking(move || edits.before(&session, &call, &command, &cwd))
                    .await
                    .ok()
                    .flatten();
            if let (Some(edited), Some(held)) = (predicted, roster.get(&hooked.session_id)) {
                held.edited(edited);
            }
        }
        Hook::PostToolUse => {
            let response = hooked.tool_response;
            let checked = tokio::task::spawn_blocking(move || edits.finished(&call, &response))
                .await
                .ok()
                .flatten();
            if let Some((session, diverged)) = checked
                && !diverged.paths.is_empty()
                && let Some(held) = roster.get(&session)
            {
                held.diverged(diverged);
            }
        }
        Hook::Other => {}
    }
    StatusCode::NO_CONTENT
}

/// `agent-console sudo`: show the command on the phone, wait for the password,
/// and run it. See [`crate::sudo`].
async fn sudo(
    State(roster): State<Arc<Roster>>,
    Json(asked): Json<crate::sudo::Request>,
) -> Json<crate::sudo::Ran> {
    let Some(session) = roster.get(&asked.session) else {
        return Json(crate::sudo::refused("no console session runs this command"));
    };
    let id = format!("sudo-{}", uuid::Uuid::new_v4());
    let answer = roster.sudo().wait(&id);
    session.ask_sudo(&id, &asked.command);
    let password = answer.await.ok().flatten();
    // Settled on the answer, not the command's end, so a long command does not
    // leave it open to a second tap.
    session.sudo_settled(&id, password.is_some());
    let ran = match password {
        Some(password) => crate::sudo::run(&asked.cwd, &asked.command, password).await,
        None => crate::sudo::refused("refused from the console"),
    };
    Json(ran)
}

/// Start signing the account in again: the link to open. See [`crate::login`].
async fn login(
    State(roster): State<Arc<Roster>>,
) -> Result<Json<crate::login::LoginStarted>, (StatusCode, String)> {
    roster
        .login()
        .start(roster.binary())
        .await
        .map(Json)
        .map_err(|why| (StatusCode::BAD_GATEWAY, why))
}

/// The code the sign-in page showed, and how the exchange went.
async fn login_code(
    State(roster): State<Arc<Roster>>,
    Json(code): Json<crate::login::LoginCode>,
) -> Result<Json<crate::login::LoginResult>, (StatusCode, String)> {
    roster
        .login()
        .finish(code)
        .await
        .map(Json)
        .map_err(|why| (StatusCode::CONFLICT, why))
}

async fn sudo_answer(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(answer): Json<crate::sudo::SudoAnswer>,
) -> StatusCode {
    if roster.sudo().answer(&answer.id, answer.password) {
        return StatusCode::NO_CONTENT;
    }
    // Nothing waits any more, as after an upgrade: clear the question.
    if let Some(session) = roster.get(&id) {
        session.sudo_settled(&answer.id, false);
    }
    StatusCode::CONFLICT
}
