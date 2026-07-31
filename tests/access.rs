//! Who may reach what, exercised through the real router.
//!
//! The history endpoints are `OwnerOnly` while the corpus is `ReadAccess`, and
//! the difference is the whole reason a share link is safe to hand out: it
//! shows somebody the memories without also handing them every prompt and
//! reply ever exchanged. That is a claim about behaviour, so it is tested
//! against the router rather than asserted from the extractor's name.
use axum::body::Body;
use axum::http::{Request, StatusCode};
use memview::access::SHARE_HEADER;
use memview::config::{AuthConfig, Config};
use memview::share::ShareStore;
use memview::state::AppState;
use tower::ServiceExt;

/// A server with auth ON — without it every request is the local owner and the
/// distinction under test does not exist.
fn app(dir: &std::path::Path) -> (AppState, String) {
    let share = ShareStore::load(dir.join("share-state.json")).expect("share store");
    let token = share.rotate().expect("rotate").token;
    let cfg = Config {
        memory_dir: dir.join("corpus").to_string_lossy().into_owned(),
        bind_addr: "127.0.0.1:0".into(),
        share_state_file: dir.join("share-state.json").to_string_lossy().into_owned(),
        public_base_url: None,
        auth: Some(AuthConfig {
            session_secret: "test-secret-not-a-real-key".into(),
            nc_base_url: "https://nextcloud.example".into(),
            nc_internal_url: None,
            nc_client_id: "id".into(),
            nc_client_secret: "secret".into(),
            nc_redirect_uri: "https://memview.example/auth/callback".into(),
            allowed_users: vec!["pippijn".into()],
        }),
        static_dir: None,
        couse_file: None,
        history_file: None,
    };
    let state = AppState::new(cfg, reqwest::Client::new(), share);
    (state, token)
}

async fn status(state: &AppState, path: &str, token: Option<&str>) -> StatusCode {
    let mut req = Request::builder().uri(path);
    if let Some(t) = token {
        req = req.header(SHARE_HEADER, t);
    }
    memview::routes::router(state.clone())
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .expect("response")
        .status()
}

#[tokio::test]
async fn a_share_token_reads_the_corpus_but_never_the_history() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("corpus")).expect("corpus dir");
    std::fs::write(dir.path().join("corpus/MEMORY.md"), "# Memory index\n").expect("index");
    let (state, token) = app(dir.path());

    // The corpus is what a share link is FOR.
    assert_eq!(
        status(&state, "/api/graph", Some(&token)).await,
        StatusCode::OK,
    );

    // The history is not. A share recipient must be refused even though their
    // token is perfectly valid — this is the property that lets a link be sent
    // to somebody without sending them 14k prompts and replies.
    assert_eq!(
        status(&state, "/api/history", Some(&token)).await,
        StatusCode::FORBIDDEN,
    );
    assert_eq!(
        status(&state, "/api/history/search?q=x", Some(&token)).await,
        StatusCode::FORBIDDEN,
    );
}

#[tokio::test]
async fn no_credential_reaches_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("corpus")).expect("corpus dir");
    std::fs::write(dir.path().join("corpus/MEMORY.md"), "# Memory index\n").expect("index");
    let (state, _token) = app(dir.path());

    for path in ["/api/graph", "/api/history", "/api/history/search?q=x"] {
        assert_eq!(
            status(&state, path, None).await,
            StatusCode::UNAUTHORIZED,
            "{path}",
        );
    }
}

#[tokio::test]
async fn a_wrong_share_token_is_not_a_viewer() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("corpus")).expect("corpus dir");
    std::fs::write(dir.path().join("corpus/MEMORY.md"), "# Memory index\n").expect("index");
    let (state, _token) = app(dir.path());

    assert_eq!(
        status(&state, "/api/graph", Some("not-the-token")).await,
        StatusCode::UNAUTHORIZED,
    );
}
