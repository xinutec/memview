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
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::sse::{Event as Sse, KeepAlive};
use axum::response::{IntoResponse, Sse as SseResponse};
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use crate::protocol::Event;
use crate::roster::Roster;
use crate::session::Summary;
use crate::trace;

pub fn router(roster: Arc<Roster>) -> Router {
    Router::new()
        .route("/api/state", get(state))
        .route("/api/sessions", post(start))
        .route("/api/sessions/{id}/input", post(input))
        .route("/api/sessions/{id}/stop", post(stop))
        .route("/api/sessions/{id}", delete(forget))
        .route("/api/sessions/{id}/events", get(events))
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
}

async fn start(
    State(roster): State<Arc<Roster>>,
    Json(body): Json<Start>,
) -> Result<Json<Summary>, (StatusCode, String)> {
    let session = roster
        .start(&body.dir)
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

async fn events(
    State(roster): State<Arc<Roster>>,
    Path(id): Path<String>,
) -> Result<SseResponse<impl Stream<Item = Result<Sse, Infallible>>>, (StatusCode, String)> {
    let session = roster
        .get(&id)
        .ok_or((StatusCode::NOT_FOUND, format!("no session {id}")))?;

    // Subscribe BEFORE reading the history, so an event that arrives between the
    // two is delivered late rather than lost.
    let live = BroadcastStream::new(session.listen());
    let history = tokio_stream::iter(session.history());

    let stream = history
        .map(Ok::<Event, ()>)
        .chain(live.map(|got| got.map_err(|_| ())))
        .map(|got| {
            let event = match got {
                Ok(event) => event,
                // The listener fell far enough behind that the channel dropped
                // events. Saying so is the only honest option: the transcript
                // this client holds now has a hole in it.
                Err(()) => Event::Trouble {
                    detail: "the console dropped events for this client".to_string(),
                },
            };
            Ok(Sse::default().json_data(event).unwrap_or_else(|err| {
                Sse::default().data(format!("{{\"kind\":\"trouble\",\"detail\":\"{err}\"}}"))
            }))
        });

    Ok(SseResponse::new(stream).keep_alive(
        // A session can sit silent for a long time while a tool runs, and a
        // silent connection is one an intermediary is entitled to close.
        KeepAlive::new().interval(Duration::from_secs(15)),
    ))
}
