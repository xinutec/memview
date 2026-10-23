//! The app's own files: the page for its routes, a 404 for a missing file, how
//! long each may be kept, and which build is being served — for the page to
//! compare with its own (`Updates`).

use axum::Router;
use axum::http::{HeaderValue, header};
use axum::response::Response;

/// The bundle's identity: the build time stamped into the entry chunk the index
/// names, which the page compares with its own (`build-info.ts`). A page restored
/// from the WebView's cache knows its own stamp; it could not know a hash.
pub fn bundle_of(dir: &str) -> Option<String> {
    let page = std::fs::read_to_string(format!("{dir}/index.html")).ok()?;
    let start = page.find("\"main-")? + 1;
    let entry = &page[start..start + page[start..].find('"')?];
    let chunk = std::fs::read_to_string(format!("{dir}/{entry}")).ok()?;
    let stamp = chunk.find("builtAt:\"")? + "builtAt:\"".len();
    Some(chunk[stamp..stamp + chunk[stamp..].find('"')?].to_string())
}

/// The app's files from `dir`: the app for a navigation the API did not answer, a
/// 404 for a missing file. See [`spa`] and [`cache_headers`].
pub fn app_files(dir: &str) -> Router {
    let index = format!("{dir}/index.html");
    Router::new()
        .fallback_service(
            tower_http::services::ServeDir::new(dir).fallback(axum::routing::any(
                move |uri: axum::http::Uri| {
                    let index = index.clone();
                    async move { spa(&index, uri.path()) }
                },
            )),
        )
        .layer(axum::middleware::from_fn(cache_headers))
}

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

/// How long a file may be reused. A hashed one is `immutable`; everything else
/// revalidates (`no-cache`): the page names the bundle, so a stale page is a stale
/// app, and with only `Last-Modified` a WebView keeps it as fresh for days. An
/// error is never kept, or the name is not asked for again; a 304 keeps what a 200
/// would say.
async fn cache_headers(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let hashed = is_content_hashed(req.uri().path());
    let mut response = next.run(req).await;
    let failed = response.status().is_client_error() || response.status().is_server_error();
    let value = if hashed && !failed {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
    response
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
