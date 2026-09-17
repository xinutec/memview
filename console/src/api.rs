//! The JSON API, and the event stream behind it.
//!
//! Reading a session is server-sent events rather than polling: an answer appears
//! while it is being written. Every stream begins with the transcript so far and
//! continues live, so a client that connects late or reconnects sees one
//! consistent record.

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

pub fn router(roster: Arc<Roster>) -> Router {
    Router::new()
        .route("/api/state", get(state))
        .route("/api/sessions", post(start))
        .route("/api/past", get(past))
        .route("/api/sessions/{id}/input", post(input))
        .route("/api/sync/drafts", get(pull_drafts).post(push_drafts))
        // The one route that needs its own body limit: axum's default 2 MB would refuse
        // an image as a bare 413 before [`crate::images::keep`] could say why.
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
        .route("/api/sessions/{id}/tasks", get(tasks))
        .route("/api/sessions/{id}/tasks/{task}", get(task))
        .route("/api/reading", get(reading))
        .route("/api/telemetry", post(trace::record))
        // The stream is compressed too: tower-http flushes the encoder when the body has
        // nothing more to give, so a live event is not held back — pinned by
        // `a live event is not held back by the compressor` in `tests/cold.rs`. Applied
        // here rather than in `main` so the tests exercise the stack the phone talks to.
        .layer(CompressionLayer::new().compress_when(SizeAbove::new(SMALL)))
        .with_state(roster)
}

/// GET /api/reading — the corpus survey, as the nightly mined it. Read per request:
/// the artefact is 7 kB. Not computed here — that is 13 seconds over 146k
/// commands, a mining job (`reader --bin reading-json`).
async fn reading() -> Result<Json<reader::reading::CorpusRead>, StatusCode> {
    let path = std::env::var("READING_FILE").unwrap_or(
        reader::home::cache("reading.json")
            .to_string_lossy()
            .into_owned(),
    );
    // A missing artefact is a 404 the view can say "not mined yet" about, never a
    // `CorpusRead` of zeroes.
    let text = std::fs::read_to_string(&path).map_err(|_| StatusCode::NOT_FOUND)?;
    serde_json::from_str(&text)
        .map(Json)
        .map_err(|_| StatusCode::NOT_FOUND)
}

/// How large a request carrying an image may be: [`crate::images::LIMIT`] plus
/// base64's third again plus the JSON, generous so the refusal that names the size
/// can still be given.
const SMALL: u64 = 1024;

const BODY_LIMIT: usize = crate::images::LIMIT * 2;

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
    /// A fingerprint of the bundle this runner is serving, when it serves one. The
    /// client reloads when it differs from the one it booted from — there is no
    /// service worker, deliberately, behind a client-certificate gate.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub bundle: Option<String>,
    /// How much of the subscription is spent, when a reading has ever arrived — see
    /// [`crate::usage`]. Absent means no reading, and the front page draws nothing: a
    /// bar at 0% is a claim.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub usage: Option<crate::usage::Reading>,
    /// What each conversation is about, by session id — written by a model, and
    /// marked as such by the client. See [`crate::gist`]. Keyed rather than folded
    /// into each session because it covers the conversations on disk too.
    pub gists: std::collections::BTreeMap<String, crate::gist::Gist>,
    /// Who is holding what — see [`crate::tasks`]. Keyed by session id for the same
    /// reason [`Self::gists`] is; the rest is Pippijn and the unassigned pile.
    pub tasks: crate::tasks::Sweep,
    /// The unsent words each conversation is holding, by session id. Carried here
    /// because the roster is already polled every five seconds, and a draft is a
    /// sentence.
    pub drafts: std::collections::BTreeMap<String, crate::drafts::Draft>,
}

/// The bundle's identity, from the bytes of the page that loads it: index.html
/// carries the hashed chunk names, so any change to the app changes this and an
/// identical rebuild does not. Read per request. Unreadable means None — the
/// desk runs `ng serve` and serves no bundle.
fn bundle(dir: Option<&str>) -> Option<String> {
    use sha2::{Digest, Sha256};
    let page = std::fs::read(format!("{}/index.html", dir?)).ok()?;
    Some(format!("{:x}", Sha256::digest(&page))[..16].to_string())
}

async fn state(State(roster): State<Arc<Roster>>) -> Json<Overview> {
    Json(Overview {
        bundle: bundle(roster.config().static_dir.as_deref()),
        // From memory, never from the network: a sleeping dashboard must not hold up the
        // list of sessions. See [`crate::usage`].
        usage: roster.usage().reading(&roster.spent()).await,
        dirs: roster
            .config()
            .dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect(),
        repos: roster.config().repos(),
        sessions: roster.list(),
        // From memory, like the usage; the writing happens on its own timer.
        gists: roster.gists(),
        drafts: roster.drafts().all(),
        // Swept per request, off the executor and off the cached marks. See [`Roster::tasks`].
        tasks: roster.tasks().await,
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
            // The session exists and did not take the message; say both, since the caller
            // now owns a session it did not expect.
            (
                StatusCode::BAD_GATEWAY,
                format!("started {} but could not send: {err:#}", session.id),
            )
        })?;
    }
    Ok(Json(session.summary()))
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
    // A receipt, because a message arriving twice leaves no other trace — one did,
    // merged by the CLI into a single message carrying the words twice. The length,
    // not the words: a log is not where a conversation belongs.
    tracing::info!(
        "{id}: accepted {} characters to send",
        body.text.chars().count()
    );
    session
        .send(&body.text)
        .await
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(session.summary()))
}

/// How far a client has already pulled.
#[derive(Debug, Deserialize)]
pub struct Since {
    /// Absent on a first pull, which then asks for everything.
    #[serde(default)]
    pub since: Option<u64>,
}

/// Every draft past the caller's checkpoint. The protocol is life's — see
/// `life/src/sync/types.rs` and its `docs/design/sync.md`; a client library drives
/// pull and push.
async fn pull_drafts(
    State(roster): State<Arc<Roster>>,
    Query(from): Query<Since>,
) -> Json<crate::drafts::PullResponse> {
    Json(roster.drafts().pull(from.since.unwrap_or(0)))
}

/// Apply a batch of edits, and answer with the ones that lost. A conflict is a
/// 200 carrying the current master, not a 409: a batch can both land and lose in
/// one request. The per-session PUT that sat beside this was removed — two ways
/// to write one map is how the draft bugs got in.
async fn push_drafts(
    State(roster): State<Arc<Roster>>,
    Json(entries): Json<Vec<crate::drafts::PushEntry>>,
) -> Json<Vec<crate::drafts::DraftDoc>> {
    Json(roster.drafts().push(entries))
}

/// A picture from the phone, with whatever is being said about it.
#[derive(Debug, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
pub struct Shown {
    /// The bytes, base64 as the API wants them — the client already has them so (a
    /// canvas hands back a data URL).
    pub data: String,
    /// What the client believes it is sending. Checked against the bytes — see
    /// [`crate::images::keep`].
    #[serde(default)]
    pub media_type: String,
    /// What was said about it. Optional: a screenshot alone is a complete message.
    #[serde(default)]
    pub text: String,
}

/// Hand back a picture that was sent to a session, so the person who took it can
/// see it too.
///
/// Not asked of the roster first: the conversation is usually one that stopped
/// days ago. [`crate::images::find`] guards it, reading only a name it could have
/// written. Cached hard: a kept picture is never rewritten.
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

/// GET /api/picture?url=… — a picture a session pointed at, fetched from here.
/// Not session-scoped: the URL is the whole of what is asked for, and a session id
/// would be decoration that reads like a guard. Why the console fetches, and why
/// that grants nothing new, is in [`crate::images::fetch`].
///
/// Not cached: this is a window onto a file that gets re-rendered, and a cache
/// would answer the second look with the first render.
async fn elsewhere(Query(asked): Query<Elsewhere>) -> Response {
    match crate::images::fetch(&asked.url).await {
        Ok(got) => (
            [
                (header::CONTENT_TYPE, got.media_type),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
            got.bytes,
        )
            .into_response(),
        // Whose fault it was, in the status as well as the sentence, for whatever reads
        // the log.
        Err(why @ crate::images::Reason::Asked(_)) => {
            (StatusCode::BAD_REQUEST, why.to_string()).into_response()
        }
        Err(why @ crate::images::Reason::Answered(_)) => {
            (StatusCode::BAD_GATEWAY, why.to_string()).into_response()
        }
    }
}

/// The URL to fetch, as it came off the query string.
#[derive(Deserialize)]
struct Elsewhere {
    url: String,
}

/// Show a session a picture. Its own route rather than a field on [`input`]: a
/// megabyte where that is a sentence, it writes a file, and it fails for reasons
/// text cannot.
async fn show(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<Shown>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    use base64::Engine as _;

    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(body.data.trim())
        .map_err(|why| {
            (
                StatusCode::BAD_REQUEST,
                format!("that image is not base64: {why}"),
            )
        })?;
    // UTC, and named so. `now_local` is refused outright in a threaded program on
    // this platform.
    let stamp = time::OffsetDateTime::now_utc()
        .format(&time::macros::format_description!(
            "[year]-[month]-[day]-[hour][minute][second]Z"
        ))
        .unwrap_or_else(|_| "image".to_string());
    let kept = crate::images::keep(
        &crate::images::images_root(),
        &id,
        &body.media_type,
        &bytes,
        &stamp,
    )
    .map_err(|why| (StatusCode::BAD_REQUEST, why))?;
    tracing::info!(
        "showing {id} a {} of {} bytes, kept at {}",
        kept.media_type,
        bytes.len(),
        kept.path.display()
    );
    session
        .show(&body.text, &kept.media_type, body.data.trim(), &kept.path)
        .await
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(session.summary()))
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
    /// What was said about a question — options picked, or words instead. Absent for
    /// every other tool, and refused if sent for one; see
    /// [`crate::session::Session::decide`] and [`console_protocol::Reply`].
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
            // Nothing said is not a reply: an ordinary approval arrives with these absent.
            Some(&body.reply).filter(|reply| !reply.is_empty()),
        )
        .await
        // CONFLICT rather than NOT_FOUND: usually the question was answered a moment
        // ago, on another screen.
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(session.summary()))
}

/// What a client asks for when changing a session's permission mode.
#[derive(serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
struct Mode {
    mode: String,
}

/// The modes the CLI declares, so an unknown one is refused here rather than by
/// a session out of sight. The 2.1.220 binary's enum, in escalation order.
const MODES: [&str; 6] = [
    "plan",
    "default",
    "dontAsk",
    "acceptEdits",
    "auto",
    "bypassPermissions",
];

async fn mode(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
    Json(body): Json<Mode>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    if !MODES.contains(&body.mode.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "{} is not a permission mode: {}",
                body.mode,
                MODES.join(", ")
            ),
        ));
    }
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    session
        .set_mode(&body.mode)
        .await
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    // Only once the session has taken it: a mode the request failed to apply must
    // not come back at the next resume — see [`crate::modes`].
    roster.remember_mode(&id, &body.mode);
    Ok(Json(session.summary()))
}

/// What to call a conversation. See [`Session::rename`].
#[derive(serde::Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(export))]
struct Renaming {
    title: String,
}

/// Rename a conversation, including one that is working. The answer is not the
/// new name: the CLI writes a `custom-title` line and the roster reads names from
/// the transcript, so the summary carries the old one until the next listing —
/// reporting the request as the state would be the console describing its intent.
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
    Ok(Json(session.summary()))
}

/// Take back a command that is waiting for the turn to end. A command no longer
/// held is not an error: two screens can look at one session, and the turn can
/// end between the chip being drawn and the tap. The summary is the honest answer.
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
    Ok(Json(session.summary()))
}

async fn stop(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    session.stop().await;
    Ok(Json(session.summary()))
}

/// Stop a session that has stopped listening and start it again on the same
/// conversation — see [`Roster::revive`]. Slow on purpose: it waits for the old
/// process to leave the process table, measured at about thirty seconds, or one
/// transcript gets two writers.
async fn revive(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .revive(&id)
        .await
        .map_err(|why| (StatusCode::CONFLICT, why))?;
    Ok(Json(session.summary()))
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
    /// The cursor from the page the reader holds — the byte offset its first line
    /// began at. Absent means the newest page. Not a count: the file grows under one,
    /// and the client counts folded entries, not events. See [`crate::past::page`].
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
///
/// `spawn_blocking`: the walk parses the whole transcript — seconds for a large
/// one — and no gate ahead of the parser survives the format; see
/// [`crate::past::landmarks`]. The first ask pays it; [`crate::marks`] keeps what
/// the walk found and extends it (memview #808).
async fn landmarks(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::past::Landmark>>, (StatusCode, String)> {
    // Through the roster, so this reads only a transcript belonging to a session
    // this console owns — the boundary every other route has.
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

/// One event on the wire, carrying its number so the browser can quote it back.
/// `EventSource` sends the last `id:` as `Last-Event-ID` on every reconnect, which
/// is what turns a dropped connection from a wipe into a gap that gets filled.
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
    /// The last sequence number this client holds. The header covers a dropped
    /// connection; this covers a page that closed the stream on purpose and still
    /// holds the transcript. A string rather than a `u64`, so an unreadable value is
    /// this function's problem: a 400 here is a session page showing nothing.
    #[serde(default)]
    after: Option<String>,
}

/// Where a client holds the transcript through, from whichever end says so. The
/// header wins when both are there — on a reconnect of a stream opened with
/// `?after=`, the URL names where the page started, the header where it got to.
/// Unparseable is absent: the honest answer is everything.
pub fn resume_from(headers: &HeaderMap, asked: Option<&str>) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| asked.and_then(|value| value.parse::<u64>().ok()))
}

/// What a client that holds nothing is sent: the end of the transcript.
///
/// Not the log: that is the resume window, hours of it on a session watched all
/// day. This is [`crate::session::Session::seed`] applied when a READER joins.
///
/// The page is read before the number is taken: number first lets an event
/// pushed in between arrive twice, and a duplicated paragraph no client can undo.
///
/// Console-only events (`busy`, `accepted`, `started`) are in no transcript and
/// not replayed — except a question, put back below, since a session blocked on
/// one nothing draws is stopped. `console/tests/provenance.rs` makes that decision
/// for every variant, so an unclassified one does not compile. `Accepted` is left
/// out: it draws *waiting to be read* against a message parked mid-turn.
///
/// `None` when there is no transcript to read.
fn cold(id: &str, session: &crate::session::Session) -> Option<(Vec<Sse>, u64)> {
    let root = crate::past::projects_root();
    let path = crate::past::transcript_of(&root, id)?;
    let page = crate::past::page(&path, None);
    if page.events.is_empty() {
        return None;
    }
    let through = session.issued();
    let earlier = page.events.len();
    // The end of the page, NOT the clock: stamped `now()` this became a fact about
    // the connection, re-dated on every open. `None` when the last line carried no
    // time.
    let ends = page.events.last().and_then(|timed| timed.at);
    // Unnumbered, so a connection dropped part-way leaves the browser asking for the
    // seed again. The number arrives once, on the `joined` that ends the page.
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
            // A reader joined; the session did not restart. The last call in this page is
            // very likely the one running now.
            restarted: false,
        },
    }));
    // After the marker, and not optional: a question is a control request no
    // transcript holds, so a seed cannot carry it — while `Summary::asked` says
    // *waiting for you* regardless; without this a blocked session showed nothing
    // to answer for ninety minutes. The same shape [`crate::session::Session::adopt`]
    // uses after ITS seed. Unnumbered: the real `Ask` is already at or below `through`.
    for (id, question) in session.asking() {
        held.push(unnumbered(
            ends,
            Event::Ask {
                id,
                call: question.call,
                tool: question.tool,
                title: question.title,
                detail: question.detail,
                input: question.input,
            },
        ));
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

    // Subscribe BEFORE reading the backlog, so an event landing between the two is
    // delivered late rather than lost; it then arrives twice, and `through` is what
    // makes the second copy recognisable.
    let live = BroadcastStream::new(session.listen());
    let backlog = session.since(after);
    // A client that holds nothing is seeded from the transcript — see [`cold`]. The
    // log stands in only when there is no transcript.
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

    // A named event, not a field on a domain one: about the connection, so
    // `onmessage` never sees it. The page listens and empties what it holds.
    let prelude = tokio_stream::iter((!backlog.resumed).then(|| {
        Sse::default()
            .event("reset")
            .data("this stream starts again from the beginning")
    }));

    let held = tokio_stream::iter(held);

    // Where the past stops, per connection: everything above is replay, everything
    // below is happening, and a replayed `turn` looks exactly like one that just
    // ended — the page read `idle` over twelve minutes of work. Named, like `reset`,
    // and deliberately NOT the `Joined` event, which lives in the log and can be
    // trimmed out from under a late client; this is emitted on every connection.
    let caught_up = tokio_stream::iter([Sse::default()
        .event("caught-up")
        .data("everything after this is happening now")]);

    let live = live.filter_map(move |got| match got {
        // Already sent as part of the backlog.
        Ok(stamped) if stamped.seq <= through => None,
        Ok(stamped) => Some(wire(stamped)),
        // The listener fell far enough behind that the channel dropped events; the
        // transcript this client holds now has a hole. Unnumbered, so the client keeps
        // quoting the last id it can vouch for.
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
        // A session can sit silent while a tool runs, and a silent connection is one an
        // intermediary may close.
        KeepAlive::new().interval(Duration::from_secs(15)),
    ))
}

/// Conversations that already exist and could be picked up, filtered to what the
/// config allows — the rest would name private work the console cannot open anyway.
async fn past(State(roster): State<Arc<Roster>>) -> Json<Vec<crate::past::Conversation>> {
    let root = crate::past::projects_root();
    let allowed: Vec<crate::past::Conversation> = crate::past::conversations(&root)
        .into_iter()
        .filter(|conversation| roster.config().resolve(&conversation.dir).is_ok())
        .collect();
    Json(allowed)
}

/// One `Bash` command, read the way the index reads it. See [`crate::parse`].
///
/// The working directory comes from the session, never the body: a caller free to
/// choose it could make this view name any file. A live session's own directory
/// where there is one, else the transcript's. Ungated on the session running, like
/// [`tasks`]: a finished conversation's command is exactly when somebody wants this.
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

/// A session's task list, without the prose. See [`crate::tasks`]. Not gated on
/// the session running: a conversation that has ended still has a list.
async fn tasks(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Json<Vec<crate::tasks::Task>> {
    Json(roster.task_list(&id).await)
}

/// What one task says, fetched when it is opened rather than with the list.
async fn task(
    State(roster): State<Arc<Roster>>,
    // The session is in the route and deliberately unused: a task belongs to the
    // service now and its number is unique. The path keeps it so bookmarks survive.
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

/// The app itself for a route the app owns, or a plain 404 for a file that is
/// not there. Answering the index for a missing font once broke the icons with
/// nothing logged anywhere.
///
/// "Looks like a file" is the last path segment carrying a dot: every asset is
/// hashed (`main-JLBKO2QH.js`) and every SPA route is a word or an id.
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
