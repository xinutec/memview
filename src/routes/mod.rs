//! HTTP routing table.

pub mod api;
pub mod auth;
pub mod telemetry;

use axum::Router;
use axum::http::{HeaderValue, Response, header};
use axum::routing::{delete, get, post};
use tower::ServiceBuilder;
use tower_http::services::fs::ServeFileSystemResponseBody;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::state::AppState;

/// How long a static response may be reused without asking again.
///
/// ⚠ **`index.html` MUST REVALIDATE, and shipping it without saying so cost a
/// deploy nobody could see.** With no `Cache-Control` at all a client falls back
/// to *heuristic* caching from `Last-Modified`, and is free to keep the document
/// for as long as it likes without ever asking again. MEASURED on `messages`
/// 2026-08-14: an Android WebView fetched the whole API — `/api/me`,
/// `/api/conversations`, a whole thread — and never once requested `main-*.js`.
/// The phone ran a build several deploys old for hours while the server had been
/// serving the new one all along.
///
/// ⚠ The symptom is "the change did not deploy", which sends you to CI, the
/// image tag, the rollout and the manifests — all of which are correct. What
/// identified it was a rendering detail that could only come from old code.
///
/// `no-cache` rather than `no-store`: it means "ask first", not "never keep", so
/// the ETag still turns the usual case into a 304 with no body.
///
/// Everything else Angular emits carries a content hash in its NAME, so a new
/// build is a new URL and the old one can never be wrong. Those are the one kind
/// of response `immutable` is honestly available for.
fn cache_control_for(res: &Response<ServeFileSystemResponseBody>) -> Option<HeaderValue> {
    let is_html = res
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/html"));
    Some(if is_html {
        HeaderValue::from_static("no-cache")
    } else {
        HeaderValue::from_static("public, max-age=31536000, immutable")
    })
}

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
        .route("/reading", get(api::reading))
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
        // ⚠ The layer wraps only the STATIC service: an API response is neither
        // a document to revalidate nor an immutable asset, and giving JSON a
        // year-long `immutable` would be the same bug pointing the other way.
        let serve = ServiceBuilder::new()
            .layer(SetResponseHeaderLayer::overriding(
                header::CACHE_CONTROL,
                cache_control_for,
            ))
            .service(serve);
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
