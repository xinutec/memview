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
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event as Sse, KeepAlive};
use axum::response::{IntoResponse, Sse as SseResponse};
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
        .route("/api/sessions/{id}/decide", post(decide))
        .route("/api/sessions/{id}/stop", post(stop))
        .route("/api/sessions/{id}", delete(forget))
        .route("/api/sessions/{id}/events", get(events))
        .route("/api/sessions/{id}/earlier", get(earlier))
        .route("/api/telemetry", post(trace::record))
        .with_state(roster)
}

/// Everything a client needs to draw the front page in one request.
#[derive(Debug, Serialize)]
pub struct Overview {
    /// Where a session may be started — these and anything inside them.
    pub dirs: Vec<String>,
    /// The repositories inside those, for the client's picker.
    pub repos: Vec<String>,
    pub sessions: Vec<Summary>,
}

async fn state(State(roster): State<Arc<Roster>>) -> Json<Overview> {
    Json(Overview {
        dirs: roster
            .config()
            .dirs
            .iter()
            .map(|dir| dir.display().to_string())
            .collect(),
        repos: roster.config().repos(),
        sessions: roster.list(),
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
    session
        .send(&body.text)
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
        .decide(&body.id, body.allow, body.why.as_deref().unwrap_or(REFUSED))
        .await
        // CONFLICT rather than NOT_FOUND: the usual cause is that the question
        // was answered a moment ago, on another screen.
        .map_err(|err| (StatusCode::CONFLICT, format!("{err:#}")))?;
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
    events: Vec<console_protocol::Event>,
    /// The cursor for the page before this one. Zero means the start of the
    /// transcript: there is nothing older.
    from: u64,
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
        .json_data(stamped.event)
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

    let stream = prelude.chain(held).chain(live).map(Ok::<Sse, Infallible>);

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
