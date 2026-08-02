//! Who may reach what, exercised through the real router.
//!
//! The share endpoints are `OwnerOnly` while the corpus is `ReadAccess`, and
//! the difference is what makes a share link safe to hand out: it shows
//! somebody the memories without also letting them rotate or revoke the link
//! they were given, or read the token out of the API. That is a claim about
//! behaviour, so it is tested against the router rather than asserted from the
//! extractor's name.
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
        doing_file: None,
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
        agents_file: None,
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
async fn a_share_token_reads_the_corpus_but_never_the_owner_surface() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("corpus")).expect("corpus dir");
    std::fs::write(dir.path().join("corpus/MEMORY.md"), "# Memory index\n").expect("index");
    let (state, token) = app(dir.path());

    // The corpus is what a share link is FOR.
    assert_eq!(
        status(&state, "/api/graph", Some(&token)).await,
        StatusCode::OK,
    );

    // The link's own management is not. A share recipient must be refused even
    // though their token is perfectly valid — otherwise the person you sent a
    // link to could revoke it, or mint themselves a fresh one after you did.
    assert_eq!(
        status(&state, "/api/share", Some(&token)).await,
        StatusCode::FORBIDDEN,
    );
    // Nor is the agent roster: a link to one memory must not also disclose the
    // shape of the work — which projects exist, and who is doing what in them.
    assert_eq!(
        status(&state, "/api/agents", Some(&token)).await,
        StatusCode::FORBIDDEN,
    );
    // /work is the roster sliced by a query, so it discloses the same thing one
    // answer at a time — and a query is exactly how someone would go looking.
    assert_eq!(
        status(&state, "/api/work?q=dhall", Some(&token)).await,
        StatusCode::FORBIDDEN,
    );
}

/// A corpus of one memory that declares the session which wrote it, plus a
/// roster in which that session is a named agent.
fn corpus_with_an_origin(dir: &std::path::Path) -> String {
    std::fs::create_dir_all(dir.join("corpus")).expect("corpus dir");
    std::fs::write(dir.join("corpus/MEMORY.md"), "# Memory index\n").expect("index");
    std::fs::write(
        dir.join("corpus/project_alpha.md"),
        "---\nname: project_alpha\nmetadata:\n  originSessionId: s1\n---\n\nBody.\n",
    )
    .expect("memory");
    let agents = memview::agents::Agents {
        doing: Default::default(),
        renames: Default::default(),
        generated: "2026-08-01T00:00:00Z".into(),
        commits: 0,
        unattributed: 0,
        agents: vec![memview::agents::Agent {
            name: "builder".into(),
            sessions: ["s1".to_string()].into_iter().collect(),
            ..Default::default()
        }],
    };
    let path = dir.join("agents.json");
    agents.save(&path).expect("roster");
    path.to_string_lossy().into_owned()
}

/// The owner's cookie, signed the way the real login does.
fn owner_cookie(secret: &str) -> String {
    let user = memview::session::UserSession {
        user_id: "pippijn".into(),
        display_name: "Pippijn".into(),
    };
    format!(
        "{}={}",
        memview::session::COOKIE_NAME,
        memview::session::create_session(secret, &user)
    )
}

async fn body_of(state: &AppState, path: &str, header: (&str, String)) -> String {
    let res = memview::routes::router(state.clone())
        .oneshot(
            Request::builder()
                .uri(path)
                .header(header.0, header.1)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(res.status(), StatusCode::OK, "{path}");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    String::from_utf8(bytes.to_vec()).expect("utf8")
}

#[tokio::test]
async fn the_owner_sees_which_agent_wrote_a_memory_and_a_share_recipient_does_not() {
    let dir = tempfile::tempdir().expect("tempdir");
    let agents_file = corpus_with_an_origin(dir.path());
    let (mut state, token) = app(dir.path());
    // `app` builds a config with no roster; this test is about resolving one.
    let mut cfg = (*state.cfg).clone();
    cfg.agents_file = Some(agents_file);
    let secret = cfg.auth.as_ref().expect("auth").session_secret.clone();
    state.cfg = std::sync::Arc::new(cfg);

    let owner = body_of(
        &state,
        "/api/memory/project_alpha",
        ("cookie", owner_cookie(&secret)),
    )
    .await;
    // The name is the whole point: a uuid says which session, not which agent.
    assert!(owner.contains(r#""agent":"builder""#), "{owner}");

    // Same memory, same body, through a valid share link — the roster must not
    // arrive attached to a memory just because the memory itself is shareable.
    let shared = body_of(
        &state,
        "/api/memory/project_alpha",
        (SHARE_HEADER, token.clone()),
    )
    .await;
    assert!(
        shared.contains("Body."),
        "share link still reads the memory"
    );
    assert!(!shared.contains("builder"), "{shared}");
    assert!(!shared.contains(r#""origin""#), "{shared}");
}

#[tokio::test]
async fn an_origin_whose_session_was_pruned_keeps_its_id() {
    // Claude Code deletes its own old sessions, so a memory outlives the
    // transcript that wrote it. Showing the raw id beats showing nothing, and
    // beats attributing it to whichever agent happens to sort first.
    let dir = tempfile::tempdir().expect("tempdir");
    corpus_with_an_origin(dir.path());
    std::fs::write(
        dir.path().join("corpus/project_alpha.md"),
        "---\nname: project_alpha\nmetadata:\n  originSessionId: s-pruned\n---\n\nBody.\n",
    )
    .expect("memory");
    let (mut state, _token) = app(dir.path());
    let mut cfg = (*state.cfg).clone();
    cfg.agents_file = Some(
        dir.path()
            .join("agents.json")
            .to_string_lossy()
            .into_owned(),
    );
    let secret = cfg.auth.as_ref().expect("auth").session_secret.clone();
    state.cfg = std::sync::Arc::new(cfg);

    let owner = body_of(
        &state,
        "/api/memory/project_alpha",
        ("cookie", owner_cookie(&secret)),
    )
    .await;
    assert!(owner.contains(r#""session":"s-pruned""#), "{owner}");
    assert!(!owner.contains(r#""agent""#), "{owner}");
}

#[tokio::test]
async fn no_credential_reaches_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(dir.path().join("corpus")).expect("corpus dir");
    std::fs::write(dir.path().join("corpus/MEMORY.md"), "# Memory index\n").expect("index");
    let (state, _token) = app(dir.path());

    for path in [
        "/api/graph",
        "/api/search?q=x",
        "/api/share",
        "/api/agents",
        "/api/work?q=x",
    ] {
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
