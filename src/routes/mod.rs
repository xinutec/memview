//! HTTP routing table.

pub mod api;
pub mod auth;
pub mod telemetry;

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::routing::{get, post};
use tower::ServiceBuilder;
use tower_http::services::ServeDir;
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::Level;

use crate::state::AppState;

/// A name carrying its content hash (`main-7GNM4EPX.js`, a font's `-ABCD1234`):
/// a new build is a new name, so it may be cached for ever.
fn is_content_hashed(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("");
    let Some((stem, _ext)) = name.rsplit_once('.') else {
        return false;
    };
    let Some((_, hash)) = stem.rsplit_once('-') else {
        return false;
    };
    hash.len() == 8
        && hash
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

/// How long a static response may be reused. A hashed file is `immutable`.
/// Everything else revalidates (`no-cache`: ask first, then a 304 via the ETag):
/// the page names the bundle, and an icon, the manifest and the service worker's
/// files keep their names across builds, so a year of `immutable` pins the old
/// one on every phone. Without `Cache-Control` a WebView caches heuristically and
/// runs a build several deploys old. An error is never `immutable`, or the name is
/// not asked for again; a 304 keeps what a 200 would say.
async fn cache_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let hashed = is_content_hashed(req.uri().path());
    let mut res = next.run(req).await;
    let failed = res.status().is_client_error() || res.status().is_server_error();
    let value = if hashed && !failed {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    res.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    res
}

/// Serve the app for a client-side route, and 404 anything naming a file (a dot
/// in the last segment) — a page served as a font fails silently.
fn spa(index: &str, path: &str) -> axum::response::Response {
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
            // STATIC_DIR set with no index is a misconfigured deployment, and
            // saying so beats serving an empty page that looks like the app.
            tracing::error!("the app's index could not be read: {error}");
            (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "no index").into_response()
        }
    }
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
        .route("/telemetry", post(telemetry::record))
        // Not the page: a client asking the API for JSON must not get HTML.
        .fallback(|| async { crate::error::AppError::NotFound });

    let app = Router::new()
        .route("/login", get(auth::login))
        .route("/auth/callback", get(auth::callback))
        .route("/logout", post(auth::logout))
        .nest("/api", api);

    // Serve the built Angular bundle (single origin), SPA-fallback to
    // index.html so deep links (/m/<name>) load the shell.
    // API-only when STATIC_DIR is unset (dev: `ng serve` proxies).
    let app = if let Some(dir) = state.cfg.static_dir.clone() {
        let index = format!("{dir}/index.html");
        let serve = ServeDir::new(&dir).fallback(get(move |uri: axum::http::Uri| {
            let index = index.clone();
            async move { spa(&index, uri.path()) }
        }));
        // Only the static service: JSON must not be `immutable`.
        let serve = ServiceBuilder::new()
            .layer(axum::middleware::from_fn(cache_headers))
            .service(serve);
        app.fallback_service(serve)
    } else {
        app
    };

    // One line per request. At INFO explicitly: `tower_http` logs at DEBUG by
    // default, which the deployment's filter hides.
    let trace = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
        .on_response(DefaultOnResponse::new().level(Level::INFO));

    // /healthz after the layer, untraced: the probes would bury real requests.
    app.layer(trace)
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state)
}
