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
    // ⚠ The defect this exists for. A font that was briefly missing — the bundle
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
