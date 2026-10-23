//! What the console answers for a path it has no file for.
//!
//! The app owns routes with no file behind them (`/s/<uuid>`), so "not found"
//! cannot simply mean 404 — but answering the app for *everything* is what made
//! a missing font silent. These pin the line between the two.

use axum::http::StatusCode;
use console::api::spa;

/// An index on disk, so the happy path is the real one rather than a stub.
fn index() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-serving-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    let path = dir.join("index.html");
    std::fs::write(&path, "<!doctype html><title>console</title>").expect("index");
    path
}

#[test]
fn a_route_the_app_owns_gets_the_app() {
    // A deep link to a session. There is no file at this path and there never
    // will be; the app reads the URL itself.
    let index = index();
    let answer = spa(
        &index.display().to_string(),
        "/s/7c0202eb-080b-40a5-a654-8758b4ca723e",
    );
    assert_eq!(answer.status(), StatusCode::OK);
}

#[test]
fn the_root_gets_the_app() {
    let index = index();
    assert_eq!(
        spa(&index.display().to_string(), "/").status(),
        StatusCode::OK
    );
}

#[test]
fn a_missing_asset_is_not_found_rather_than_the_app() {
    // The defect this exists for. A font that was briefly missing — the bundle
    // is rewritten in place on every build — came back as 200 text/html, and a
    // browser given HTML where it asked for a font shows broken icons and
    // reports nothing anywhere.
    let index = index();
    let answer = spa(
        &index.display().to_string(),
        "/media/material-icons-GONEGONE.woff2",
    );
    assert_eq!(answer.status(), StatusCode::NOT_FOUND);
}

#[test]
fn every_shape_of_asset_the_bundle_asks_for_is_covered() {
    // The real names, because the rule is a heuristic about them: hashed files
    // for assets, words and ids for routes.
    let index = index().display().to_string();
    for asset in [
        "/main-JLBKO2QH.js",
        "/styles-YGANRVWE.css",
        "/media/material-icons-LEZCGFVT.woff2",
        "/media/roboto-cyrillic-300-normal-3BUT4JGL.woff",
    ] {
        assert_eq!(
            spa(&index, asset).status(),
            StatusCode::NOT_FOUND,
            "{asset} must 404 when it is missing, not answer the app"
        );
    }
}

#[test]
fn a_missing_index_says_so_rather_than_serving_nothing() {
    // A console pointed at a directory that has no app in it. Silence here would
    // read as a blank page with no explanation.
    let answer = spa("/nowhere/at/all/index.html", "/");
    assert_eq!(answer.status(), StatusCode::INTERNAL_SERVER_ERROR);
}

/// The app's files served by the console's own router, on a free port.
async fn served() -> String {
    let dir = index().parent().expect("its directory").to_path_buf();
    std::fs::write(dir.join("main-JLBKO2QH.js"), "console.log(1)").expect("a chunk");
    let app = console::api::app_files(&dir.display().to_string());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a port");
    let at = listener.local_addr().expect("an address");
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{at}")
}

async fn cache_control(url: &str) -> Option<String> {
    // The client is built with no TLS provider of its own; plain HTTP still asks.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let answer = reqwest::get(url).await.expect("an answer");
    assert_eq!(answer.status(), StatusCode::OK, "{url}");
    answer
        .headers()
        .get("cache-control")
        .map(|value| value.to_str().expect("text").to_string())
}

#[tokio::test]
async fn the_app_page_is_asked_for_again_every_time() {
    // The page names the bundle, so a stale copy is a stale app. Served with
    // only `Last-Modified`, the phone's WebView kept a days-old page as fresh and
    // booted the build before last on a cold start — and the self-update took
    // that build for the one it booted from, so it never reloaded.
    let at = served().await;
    assert_eq!(
        cache_control(&format!("{at}/")).await.as_deref(),
        Some("no-cache")
    );
    assert_eq!(
        cache_control(&format!("{at}/s/7c0202eb-080b-40a5-a654-8758b4ca723e"))
            .await
            .as_deref(),
        Some("no-cache")
    );
    // A hashed chunk never changes under its name: no reason to ask again.
    assert_eq!(cache_control(&format!("{at}/main-JLBKO2QH.js")).await, None);
}

/// A bundle as `ng build` leaves it: an index naming its entry chunk, and the
/// chunk carrying the stamp `build-info.ts` compiles to.
fn bundle_dir(name: &str, chunk: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("console-bundle-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch");
    std::fs::write(
        dir.join("index.html"),
        r#"<!doctype html><script src="main-XFMIDOIB.js" type="module"></script>"#,
    )
    .expect("index");
    std::fs::write(dir.join("main-XFMIDOIB.js"), chunk).expect("chunk");
    dir
}

#[test]
fn the_bundle_is_named_by_the_build_stamp_the_page_also_carries() {
    // What the page compares with its own `BUILD_INFO.builtAt`. A hash of the
    // index was something no page could know about itself, so a page restored
    // from the WebView's cache took the new hash for its own and never reloaded.
    let dir = bundle_dir(
        "stamped",
        r#"var r gS={sha:"7805403",builtAt:"2026-10-23T09:34:13.521Z"};var x"#,
    );
    assert_eq!(
        console::api::bundle_of(&dir.display().to_string()).as_deref(),
        Some("2026-10-23T09:34:13.521Z")
    );
}

#[test]
fn a_bundle_with_no_stamp_is_no_bundle() {
    // Never a fallback fingerprint: no page carries one to match, so every page
    // would reload, forever.
    let dir = bundle_dir("unstamped", "console.log(1)");
    assert_eq!(console::api::bundle_of(&dir.display().to_string()), None);
}
