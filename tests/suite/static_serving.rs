//! **A missing FILE must 404, not be handed the page.**
//!
//! #1478, measured across the fleet 2026-09-08: `GET /media/nope.woff2` came
//! back `200 text/html` — the SPA shell, to a browser that asked for a font. It
//! renders broken icons and reports nothing at all, so the failure is silent on
//! both sides, and the wrong answer being a `200` is what makes it invisible.
//!
//! ⚠ **This app is not the console.** `console/src/api.rs` has carried `spa()`
//! since it hit the same defect; that is a different binary, and the fix never
//! reached the one serving `memview.xinutec.org`. The task that recorded
//! "memview was fixed the same way" was right about the console and wrong about
//! here — worth knowing, because it reads as covering both.
//!
//! The rule is a dot in the last path segment: `/m/some-memory` is a route and
//! `/main-ABC123.js` is a file. A heuristic, and the alternative — enumerating
//! the bundle's own asset names — would have to be rebuilt whenever `ng build`
//! changes a hash.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use memview::config::Config;
use memview::routes;
use memview::share::ShareStore;
use memview::state::AppState;
use tower::ServiceExt;

/// A static dir shaped like a real `ng build` output, plus the state files the
/// server insists on.
fn app(dir: &std::path::Path) -> AppState {
    let static_dir = dir.join("static");
    std::fs::create_dir_all(&static_dir).expect("create static dir");
    std::fs::write(
        static_dir.join("index.html"),
        "<!doctype html><html></html>",
    )
    .expect("index");
    std::fs::write(static_dir.join("main-ABC123.js"), "export {};").expect("bundle");
    let share = ShareStore::load(dir.join("share-state.json")).expect("share store");
    let cfg = Config {
        doing_file: None,
        effects_file: None,
        reading_file: None,
        memory_dir: dir.join("corpus").to_string_lossy().into_owned(),
        bind_addr: "127.0.0.1:0".into(),
        share_state_file: dir.join("share-state.json").to_string_lossy().into_owned(),
        public_base_url: None,
        // Auth OFF: every request is the local owner, which is what lets these
        // reach the static service instead of a login redirect.
        auth: None,
        static_dir: Some(static_dir.to_string_lossy().into_owned()),
        couse_file: None,
        agents_file: None,
    };
    AppState::new(cfg, reqwest::Client::new(), share)
}

async fn get(path: &str) -> (StatusCode, String) {
    let dir = std::env::temp_dir().join(format!(
        "memview-serving-{}-{}",
        std::process::id(),
        path.replace('/', "_")
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let res = routes::router(app(&dir))
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let ct = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .map(|v| v.to_str().unwrap().to_owned())
        .unwrap_or_default();
    let _ = std::fs::remove_dir_all(&dir);
    (status, ct)
}

#[tokio::test]
async fn a_missing_asset_is_a_404_and_not_the_page() {
    let (status, ct) = get("/media/nope.woff2").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(
        !ct.starts_with("text/html"),
        "a font request got HTML: {ct}"
    );
}

/// The other half, and the one a careless fix breaks: a client-side route has
/// no dot and must still load the shell, or every deep link 404s.
#[tokio::test]
async fn a_deep_link_still_gets_the_page() {
    let (status, ct) = get("/m/some-memory").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        ct.starts_with("text/html"),
        "a route did not get the page: {ct}"
    );
}

/// And a file that EXISTS is still served as itself.
#[tokio::test]
async fn a_real_asset_is_still_served() {
    let (status, ct) = get("/main-ABC123.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !ct.starts_with("text/html"),
        "the bundle came back as HTML: {ct}"
    );
}
