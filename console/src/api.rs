//! The JSON API, and the event stream behind it.
//!
//! Reading a session is server-sent events rather than polling: the whole point
//! of the console is that an answer appears while it is being written, and a
//! poll interval is a lower bound on how stale the screen is.
//!
//! Every stream begins with the transcript so far and continues live, so a
//! client that connects late, reconnects on a dropped train connection, or opens
//! a second window sees one consistent record rather than a fragment.

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
        // ⚠ **The one route that needs its own body limit.** Axum's default is
        // 2 MB, and an image is the only thing this API takes that is bigger than
        // a sentence — without this the limit is enforced by the framework, as a
        // bare 413 with nothing to say, before any of the reasons in
        // [`crate::images::keep`] can be given.
        .route(
            "/api/sessions/{id}/image",
            post(show).layer(axum::extract::DefaultBodyLimit::max(BODY_LIMIT)),
        )
        .route("/api/sessions/{id}/images/{name}", get(picture))
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
        .route("/api/telemetry", post(trace::record))
        .with_state(roster)
}

/// How large a request carrying an image may be.
///
/// [`crate::images::LIMIT`] is the picture; this is the request around it, so it
/// has to allow for base64's third again plus the JSON. Generous rather than
/// exact: the useful refusal is the one that names the size in megabytes, and
/// that one cannot be given if the framework has already dropped the body.
const BODY_LIMIT: usize = crate::images::LIMIT * 2;

/// Everything a client needs to draw the front page in one request.
#[derive(Debug, Serialize)]
pub struct Overview {
    /// Where a session may be started — these and anything inside them.
    pub dirs: Vec<String>,
    /// The repositories inside those, for the client's picker.
    pub repos: Vec<String>,
    pub sessions: Vec<Summary>,
    /// A fingerprint of the bundle this runner is serving, when it serves one.
    ///
    /// The client compares it with the one it booted from and reloads when they
    /// differ — the console has no service worker (deliberately: it would cache
    /// an app behind a client-certificate gate, and ngsw's navigationUrls and
    /// auth are a known source of trouble here), so nothing else would ever
    /// tell a long-lived page that the bundle under it had changed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    /// How much of the subscription is spent, when a reading has ever arrived.
    ///
    /// Measured from the sessions' own streams, with the home dashboard behind
    /// it for a window nothing has reported yet — see [`crate::usage`]. Absent
    /// means no reading rather than no usage, and the front page then shows
    /// nothing at all: a bar drawn at 0% is a claim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<crate::usage::Reading>,
    /// What each conversation is about, by session id — written by a model from
    /// the transcript rather than read off it, and marked as such by the client.
    /// See [`crate::gist`].
    ///
    /// Keyed rather than folded into each session because it covers the
    /// conversations on disk too, which arrive from a different endpoint and are
    /// the ones a sentence helps most: a name you have not opened in a week is a
    /// word, and this says what the week's work was.
    pub gists: std::collections::BTreeMap<String, crate::gist::Gist>,
    /// Who is holding what — see [`crate::tasks`]. The conversations are keyed
    /// by session id; the rest is Pippijn and the unassigned pile.
    ///
    /// Keyed for the same reason [`Self::gists`] is, and it is the same reason
    /// twice: the front page draws the transcripts on disk beside the running
    /// sessions, and a conversation that is not running still has the list it
    /// kept. A copy folded onto each summary would cover only half the page.
    pub tasks: crate::tasks::Sweep,
}

/// The bundle's identity, from the bytes of the page that loads it.
///
/// index.html is rewritten on every build with the hashed filenames of the
/// entry chunks, so any change to the app changes this — and a rebuild that
/// produces identical output does not, which is why this hashes content rather
/// than reading mtime. Read per request because the point is to notice a
/// rebuild that happened while the runner kept running.
///
/// Unreadable means None rather than an error: a runner serving no bundle at
/// all is a normal configuration (the desk runs `ng serve`), and a missing
/// fingerprint simply means the client never decides it is stale.
fn bundle(dir: Option<&str>) -> Option<String> {
    use sha2::{Digest, Sha256};
    let page = std::fs::read(format!("{}/index.html", dir?)).ok()?;
    Some(format!("{:x}", Sha256::digest(&page))[..16].to_string())
}

async fn state(State(roster): State<Arc<Roster>>) -> Json<Overview> {
    Json(Overview {
        bundle: bundle(roster.config().static_dir.as_deref()),
        // From memory, never from the network: this handler answers the front
        // page's poll, and a dashboard that has gone to sleep must not be able
        // to hold up the list of sessions. The live half costs a walk over the
        // sessions' tallies. See [`crate::usage`].
        usage: roster.usage().reading(&roster.spent()).await,
        dirs: roster
            .config()
            .dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect(),
        repos: roster.config().repos(),
        sessions: roster.list(),
        // From memory like the usage, and for the same reason: this handler is
        // the front page's five-second poll. The writing happens on its own
        // timer.
        gists: roster.gists(),
        // Swept per request rather than held, because two numbers that go stale
        // are worse than no numbers — but off the executor, and off the cached
        // marks. See [`Roster::tasks`].
        tasks: roster.tasks().await,
    })
}

#[derive(Debug, Deserialize)]
pub struct Start {
    pub dir: String,
    /// The first instruction. Optional: a session can be opened and then talked
    /// to, which is what starting one from the phone before deciding what to ask
    /// looks like.
    #[serde(default)]
    pub prompt: Option<String>,
    /// A conversation to pick up rather than starting a new one. Its id is kept,
    /// so the console's handle and the transcript stay the same thing.
    #[serde(default)]
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
            // The session exists and did not take the message; say both, since
            // the caller now owns a session it did not expect to have.
            (
                StatusCode::BAD_GATEWAY,
                format!("started {} but could not send: {err:#}", session.id),
            )
        })?;
    }
    Ok(Json(session.summary()))
}

#[derive(Debug, Deserialize)]
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
    // ⚠ **A receipt, because a message arriving twice leaves no other trace.**
    // One did: the same words reached the CLI inside a millisecond and were
    // merged into a single message carrying them twice, and nothing on this
    // machine could say whether the phone had sent once or twice. The length
    // rather than the words — this is enough to count arrivals, and a log is not
    // where a conversation belongs.
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

/// A picture from the phone, with whatever is being said about it.
#[derive(Debug, Deserialize)]
pub struct Shown {
    /// The bytes, base64 as the API itself wants them — the client has them in
    /// that form already (a canvas hands back a data URL), so decoding them to
    /// re-encode them at the far end would be work done twice.
    pub data: String,
    /// What the client believes it is sending. Checked against the bytes rather
    /// than believed — see [`crate::images::keep`].
    #[serde(default)]
    pub media_type: String,
    /// What was said about it. Optional: a screenshot sent with nothing said is
    /// a complete message, and the commonest one.
    #[serde(default)]
    pub text: String,
}

/// Hand back a picture that was sent to a session.
///
/// The other half of showing one. Without it the person who took the screenshot
/// is the only party to the conversation who cannot see it: the model gets the
/// image, and the transcript on the phone had a sentence about a file path.
///
/// ⚠ **Not asked of the roster first.** Every other `/api/sessions/{id}` route
/// wants a session that is running; this one wants a file, and the conversation
/// it belongs to is usually one that stopped days ago — which is exactly when
/// somebody scrolls back to look. What guards it instead is
/// [`crate::images::find`], which will only read a name it could have written.
///
/// Cached hard: a kept picture is written once under a name carrying the second
/// it arrived in, and is never rewritten. Without this the phone re-fetches every
/// screenshot in a conversation on every scroll back through it.
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

/// Show a session a picture.
///
/// Its own route rather than a field on [`input`], because the two are different
/// requests in every practical sense: this one is a megabyte where that one is a
/// sentence, it writes a file, and it can fail for reasons — too large, not an
/// image — that have no meaning for text.
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
    // this platform, and a filename that silently means one of two timezones is
    // worse than one that plainly means the other.
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
pub struct Decision {
    /// The control-request id from the `ask` event.
    pub id: String,
    pub allow: bool,
    /// Why not. Ignored on an allow; the session is told it on a refusal.
    #[serde(default)]
    pub why: Option<String>,
    /// What was said about a question — options picked, or words instead. Absent
    /// for every other tool, and refused if sent for one; see
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
            // Nothing said is not a reply: an ordinary approval of any tool
            // arrives here with these fields absent, and must stay one.
            Some(&body.reply).filter(|reply| !reply.is_empty()),
        )
        .await
        // CONFLICT rather than NOT_FOUND: the usual cause is that the question
        // was answered a moment ago, on another screen.
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
    Ok(Json(session.summary()))
}

/// What a client asks for when changing a session's permission mode.
#[derive(serde::Deserialize)]
struct Mode {
    mode: String,
}

/// The modes the CLI declares, so an unknown one is refused here rather than
/// sent to a session that will reject it out of sight.
///
/// The 2.1.220 binary's own enum, in its escalation order — `plan` lets least
/// through, `bypassPermissions` most.
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
    // Only once the session has taken it. Remembering a mode the request failed
    // to apply would put the console back on it at the next resume, which is the
    // one direction this must never get wrong — see [`crate::modes`].
    roster.remember_mode(&id, &body.mode);
    Ok(Json(session.summary()))
}

/// What to call a conversation. See [`Session::rename`].
#[derive(serde::Deserialize)]
struct Renaming {
    title: String,
}

/// Rename a conversation, including one that is working.
///
/// ⚠ **The answer is not the new name.** The CLI writes a `custom-title` line to
/// the transcript and the roster reads every name from there, so the summary
/// returned here still carries the old one until the next listing reads the
/// file. That is deliberate: reporting the requested name as the session's would
/// be the console describing its own intent again.
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

/// Take back a command that is waiting for the turn to end.
///
/// ⚠ **A command that is no longer held is not an error.** Two screens can be
/// looking at one session, and the turn can end between the chip being drawn and
/// the tap on it — in both cases the honest answer is the session as it now is,
/// which is what the summary says. Failing would report a mistake to somebody
/// who did not make one.
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
/// conversation. See [`Roster::revive`] — including why the unread messages have
/// to be handed back.
///
/// ⚠ **Slow on purpose, and the client has to expect that.** It waits for the
/// old process to actually leave the process table, which has been measured at
/// about thirty seconds, because resuming before it does gives one transcript
/// two writers.
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

/// What was said before the page the reader already has.
///
/// The seed is a page, not the conversation: scrolling to the top of it used to
/// be the end of the road, on a transcript that might hold a thousand turns
/// below it. This is the next page back.
///
/// Reads the file rather than the session's log, and it is the same reader — so
/// a conversation reads the same whether it arrived through the seed or through
/// scrolling.
#[derive(serde::Deserialize)]
struct Earlier {
    /// The cursor from the page the reader already holds — the byte offset its
    /// first line began at. Absent means the newest page.
    ///
    /// ⚠ Not a count of what the reader has. That is what this was, and it was
    /// wrong in both directions: the file grows under a count taken from its
    /// end, and the client counts folded entries rather than events, so the
    /// number never meant what the server read it as. See [`crate::past::page`].
    #[serde(default)]
    before: Option<u64>,
}

#[derive(serde::Serialize)]
struct Page {
    events: Vec<console_protocol::Timed>,
    /// The cursor for the page before this one. Zero means the start of the
    /// transcript: there is nothing older.
    from: u64,
}

/// Everywhere in this conversation worth jumping to.
///
/// ⚠ **`spawn_blocking`, and this is the one route that has earned it.** The
/// walk parses the whole transcript — 0.7 s for an ordinary large one and 3.4 s
/// for the biggest here, measured — and no gate ahead of the parser survives
/// contact with the format; see [`crate::past::landmarks`]. Left on the executor
/// it would be seconds of a worker that every other session's stream shares, for
/// one person tapping "go to".
async fn landmarks(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<crate::past::Landmark>>, (StatusCode, String)> {
    // Through the roster, so this can only read a transcript belonging to a
    // session this console owns — the same boundary every other route has.
    roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;
    let root = crate::past::projects_root();
    let path = crate::past::transcript_of(&root, &id).ok_or((
        StatusCode::NOT_FOUND,
        format!("no transcript on disk for {id}"),
    ))?;

    let began = std::time::Instant::now();
    let found = tokio::task::spawn_blocking(move || crate::past::landmarks(&path))
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
    // Through the roster, so this can only read a transcript belonging to a
    // session this console owns — the same boundary every other route has.
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
///
/// `EventSource` remembers the last `id:` it saw and sends it as `Last-Event-ID`
/// on every reconnect, with no help from the page. That is the whole mechanism:
/// numbering the events is what turns a dropped connection from a wipe into a
/// gap that gets filled.
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
    /// The last sequence number this client holds.
    ///
    /// The header is the browser's business and covers a dropped connection.
    /// This covers the other case, which the header cannot: a page that closed
    /// the stream on purpose — navigating away from a session and back — and
    /// still holds the transcript it read. `EventSource` sends `Last-Event-ID`
    /// only for its own automatic reconnects, so a brand new one arrives
    /// claiming nothing and would be sent the conversation all over again.
    ///
    /// A string rather than a `u64` so that a value we cannot read is *this
    /// function's* problem rather than the extractor's: typed, axum rejects the
    /// request, and a 400 here is a session page showing nothing at all — a far
    /// worse answer to a bad number than sending the transcript.
    #[serde(default)]
    after: Option<String>,
}

/// Where a client holds the transcript through, from whichever end says so.
///
/// The header wins when both are there, and both are there on every reconnect of
/// a stream opened with `?after=`: the URL goes on naming where the page started
/// while the header names where it got to.
///
/// Unparseable is absent, at both ends. A client we cannot understand gets the
/// honest answer, which is everything.
pub fn resume_from(headers: &HeaderMap, asked: Option<&str>) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .or_else(|| asked.and_then(|value| value.parse::<u64>().ok()))
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

    // Subscribe BEFORE reading the backlog, so an event landing between the two
    // is delivered late rather than lost. It also arrives *twice* — it is in the
    // snapshot and in the channel — which went unnoticed while events were
    // anonymous, and showed up as a duplicated paragraph. `through` is what makes
    // the second copy recognisable.
    let live = BroadcastStream::new(session.listen());
    let backlog = session.since(after);
    let through = backlog.through;
    tracing::info!(
        "{id}: {} events for a reader at {after:?} ({})",
        backlog.events.len(),
        if backlog.resumed {
            "resumed"
        } else {
            "from the top"
        }
    );

    // A named event rather than a field on a domain one: this is about the
    // connection, not about the session, and `onmessage` never sees it. The page
    // listens for it and empties what it holds — the only time it now has to.
    let prelude = tokio_stream::iter((!backlog.resumed).then(|| {
        Sse::default()
            .event("reset")
            .data("this stream starts again from the beginning")
    }));

    let held = tokio_stream::iter(backlog.events).map(wire);

    // ⚠ **Where the past stops, said per connection rather than per session.**
    // Everything above is a replay; everything below is happening. A client
    // cannot tell them apart otherwise — a replayed `turn` looks exactly like a
    // turn that just ended — and it *must*, because the CLI announces a status
    // only when it changes: a session already working when somebody joined said
    // nothing further until it stopped, so the replayed `turn` at the end of the
    // backlog stood as the client's last word on the matter and the page read
    // `idle` over twelve minutes of work.
    //
    // Named, like `reset`, and for the same reason: it is a fact about this
    // stream, not about the conversation, so `onmessage` never sees it and no
    // transcript entry comes of it. Deliberately NOT the `Joined` event, which
    // is in the log and can therefore be trimmed out from under a client that
    // connects late — this one is emitted on every connection by construction.
    let caught_up = tokio_stream::iter([Sse::default()
        .event("caught-up")
        .data("everything after this is happening now")]);

    let live = live.filter_map(move |got| match got {
        // Already sent as part of the backlog.
        Ok(stamped) if stamped.seq <= through => None,
        Ok(stamped) => Some(wire(stamped)),
        // The listener fell far enough behind that the channel dropped events.
        // Saying so is the only honest option: the transcript this client holds
        // now has a hole in it. Deliberately unnumbered, so the client keeps
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
        // A session can sit silent for a long time while a tool runs, and a
        // silent connection is one an intermediary is entitled to close.
        KeepAlive::new().interval(Duration::from_secs(15)),
    ))
}

/// Conversations that already exist and could be picked up.
///
/// Filtered to what the config allows, because a list of every directory this
/// machine has ever run a session in is not the console's to hand out — it would
/// name private work the console cannot open anyway.
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
/// ⚠ **The working directory comes from the session, never from the body.** A
/// relative operand resolves against it, so a caller free to choose it could
/// make this view name any file it liked — and the whole worth of the view is
/// that it says what the miner would say. A live session's own directory is
/// used where there is one; otherwise the conversation's transcript is asked,
/// which is the same answer one turn staler.
///
/// Ungated on the session being one this console runs, like [`tasks`]: reading a
/// finished conversation's command is exactly when somebody wants this.
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

/// A session's task list, without the prose. See [`crate::tasks`].
///
/// Not gated on the session being one this console runs: the list belongs to the
/// conversation, and a conversation that has ended still has one worth reading.
async fn tasks(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Json<Vec<crate::tasks::Listed>> {
    Json(roster.task_list(&id).await)
}

/// What one task says, fetched when it is opened rather than with the list.
async fn task(
    State(roster): State<Arc<Roster>>,
    // ⚠ The session is in the route and deliberately unused: a task belongs to
    // the service now, and its number is unique across every conversation. The
    // path keeps the session so the client's URLs — and anything bookmarked —
    // stay what they were.
    Path((_session, task)): Path<(String, String)>,
) -> impl IntoResponse {
    match roster.task_detail(&task).await {
        Some(description) => Json(Described { description }).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Debug, Serialize)]
struct Described {
    description: String,
}

/// The app itself, for a route the app owns — or a plain 404 for a file that is
/// not there.
///
/// ⚠ **The console used to answer the index for everything it could not find.**
/// The bundle is rewritten in place on every build, so a file that was missing
/// for a second came back as `200 text/html`, and a browser handed HTML where it
/// asked for a font neither retries nor complains: the icons vanished on a
/// reload and nothing recorded a failure — not the server log, not the client
/// trace, not the network panel. A 404 is the answer that can be seen.
///
/// "Looks like a file" is the last path segment carrying a dot. It is a
/// heuristic, and the right one here: every asset this bundle asks for is hashed
/// (`main-JLBKO2QH.js`, `media/material-icons-LEZCGFVT.woff2`) while every route
/// the SPA owns is a word or an id (`/`, `/s/<uuid>`). A route with a dot in it
/// would 404 wrongly; there are none, and inventing one would be the bug.
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
