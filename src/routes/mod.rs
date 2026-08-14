//! HTTP routing table.

pub mod api;
pub mod auth;
pub mod telemetry;

use axum::Router;
use axum::routing::{delete, get, post};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/me", get(api::me))
        .route("/index", get(api::index))
        .route("/memories", get(api::memories))
        .route("/memory/{name}", get(api::memory))
        .route("/graph", get(api::graph))
        .route("/agents", get(api::agents))
        .route("/search", get(api::search))
        .route("/work", get(api::work))
        .route("/doing", get(api::doing))
        .route("/effects", get(api::effects))
        .route("/share", get(api::share_get))
        .route("/share", post(api::share_rotate))
        .route("/share", delete(api::share_revoke))
        .route("/telemetry", post(telemetry::record));

    let app = Router::new()
        .route("/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/logout", post(auth::logout))
        .nest("/api", api);

    // Serve the built Angular bundle (single origin), SPA-fallback to
    // index.html so deep links (/m/<name>, /share/<token>) load the shell.
    // API-only when STATIC_DIR is unset (dev: `ng serve` proxies).
    let app = if let Some(dir) = state.cfg.static_dir.clone() {
        let serve = ServeDir::new(&dir).fallback(ServeFile::new(format!("{dir}/index.html")));
        app.fallback_service(serve)
    } else {
        app
    };

    // One line per request: method, path, status, latency. The app had none, so
    // the only evidence it had ever served anything was that it was still
    // running — no way to tell a slow corpus read from a client that gave up, or
    // to see a 401 storm from a stale cookie.
    //
    // The levels are set explicitly rather than left at the defaults. TraceLayer
    // logs under the `tower_http` target, and the deployment's filter is
    // `info,memview=debug` — which raises *this crate* to debug and leaves
    // tower_http at info. Taking the default DEBUG therefore shipped a layer that
    // could never emit a line, in the name of adding observability. Responses at
    // INFO make the useful half independent of a filter string maintained in a
    // different repository.
    let trace = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO));

    // /healthz is registered AFTER the layer, so it is deliberately untraced.
    // kubelet probes it twice on two schedules — roughly three times every twenty
    // seconds, about 26,000 lines a day — and logging that buries the handful of
    // requests a person actually made. A log nobody can skim is not observability,
    // and the first version of this shipped with the probes in it.
    app.layer(trace)
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}
