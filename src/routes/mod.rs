//! HTTP routing table.

pub mod api;
pub mod auth;

use axum::Router;
use axum::routing::{delete, get, post};
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use crate::state::AppState;

pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/me", get(api::me))
        .route("/index", get(api::index))
        .route("/memories", get(api::memories))
        .route("/memory/{name}", get(api::memory))
        .route("/graph", get(api::graph))
        .route("/search", get(api::search))
        .route("/share", get(api::share_get))
        .route("/share", post(api::share_rotate))
        .route("/share", delete(api::share_revoke));

    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
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
    // At debug rather than info: the corpus is re-read per request, so a browsing
    // session is chatty, and a log that scrolls is one nobody reads. RUST_LOG is
    // already set to `info,memview=debug` in the deployment.
    app.layer(TraceLayer::new_for_http()).with_state(state)
}
