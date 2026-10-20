//! The post-login redirect target, which is attacker-settable by design.
//!
//! `?return_to=` is a query parameter on `/login`, so whatever it holds reaches
//! `Location:` after a successful sign-in. The only thing standing between a
//! crafted link and an off-site redirect is [`validate_return_to`], which makes
//! the vectors below the point of this file rather than a flourish: each is a
//! published way of spelling an absolute URL that still begins with a slash.

use memview::routes::auth::validate_return_to;

#[test]
fn keeps_an_internal_path_whole() {
    for path in [
        "/",
        "/memory/project_thing",
        "/search?q=needle",
        "/search?q=a+b&sort=rank",
        // A backslash past the first character cannot open an authority, so it
        // is an ordinary path character and the return survives it.
        "/search?q=a%5Cb",
        r"/search?q=a\b",
    ] {
        assert_eq!(validate_return_to(Some(path)), path, "{path:?} is internal");
    }
}

#[test]
fn refuses_every_spelling_that_leaves_the_site() {
    for path in [
        // Protocol-relative: the classic.
        "//attacker.example",
        "//attacker.example/path",
        // The URL Standard treats `\` as `/` for http(s), so a browser reads
        // this as `//attacker.example` and navigates off-site. It starts with a
        // slash and not with `//`, which is exactly why a `!starts_with("//")`
        // test passed it through.
        r"/\attacker.example",
        r"/\/attacker.example",
        r"\\attacker.example",
        // Absolute, in the forms that do not begin with a slash at all.
        "https://attacker.example",
        "http://attacker.example",
        r"\/attacker.example",
        // No scheme-relative form survives either.
        "attacker.example",
        "javascript:alert(1)",
    ] {
        assert_eq!(
            validate_return_to(Some(path)),
            "/",
            "{path:?} must not be a redirect target"
        );
    }
}

#[test]
fn an_absent_or_empty_target_is_the_root() {
    assert_eq!(validate_return_to(None), "/");
    assert_eq!(validate_return_to(Some("")), "/");
}

/// Which pending sign-in a callback answers, when the identity provider may or
/// may not hand back the `state` it was given.
///
/// This is the security-relevant half of the sign-in and it is a pure
/// function precisely so it can be pinned here: the live flow needs Nextcloud,
/// which no test has, and a decision nobody can exercise is a decision nobody
/// checks.
mod which_signin {
    use memview::routes::auth::{Answer, state_to_consume};

    #[test]
    fn the_url_is_used_when_the_provider_hands_it_back() {
        // An identity provider that behaves: cookie and URL agree, and either
        // would do. This is the path every other deployment takes.
        assert_eq!(state_to_consume("abc", "abc"), Answer::Exchange("abc"));
        // No cookie: a state this browser never carried. `/login` mints one
        // for anyone, so finding it in the store proves nothing (login CSRF).
        assert!(matches!(state_to_consume("abc", ""), Answer::Refuse(_)));
    }

    /// ⚠ **The live case, and never trusted (RFC 6749 §10.12).** Nextcloud is
    /// handed 48 hex characters and can return `state=`. Then nothing ties the
    /// URL's code to this browser: anyone's code would do. The cookie names
    /// the sign-in to retry, never one to finish.
    #[test]
    fn a_dropped_state_is_retried_never_exchanged() {
        assert_eq!(state_to_consume("", "abc"), Answer::Retry("abc"));
    }

    /// Refused, not resolved. A cookie proves this browser began SOME
    /// flow; when the URL names a different one, nothing here can say which is
    /// meant, and picking either would be a guess about an authentication.
    #[test]
    fn two_that_disagree_are_refused_rather_than_guessed() {
        let Answer::Refuse(said) = state_to_consume("abc", "xyz") else {
            panic!("must not resolve");
        };
        assert!(said.contains("different sign-in"), "{said:?}");
    }

    #[test]
    fn neither_present_is_refused() {
        let Answer::Refuse(said) = state_to_consume("", "") else {
            panic!("must not resolve");
        };
        assert!(said.contains("did not start here"), "{said:?}");
    }

    /// Every refusal must be something a person can act on, because it is
    /// rendered as the whole page they are looking at.
    #[test]
    fn every_refusal_is_a_sentence_not_a_code() {
        for (url, cookie) in [("", ""), ("abc", "xyz")] {
            let Answer::Refuse(said) = state_to_consume(url, cookie) else {
                panic!("a refusal");
            };
            assert!(said.ends_with('.'), "{said:?} should be a sentence");
            assert!(said.len() > 20, "{said:?} is too terse to act on");
        }
    }
}

/// The callback itself, through the router: what a browser is sent when
/// Nextcloud drops `state`.
mod retry {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use memview::config::{AuthConfig, Config};
    use memview::state::AppState;
    use tower::ServiceExt;

    pub(super) fn app() -> axum::Router {
        let cfg = Config {
            doing_file: None,
            effects_file: None,
            reading_file: None,
            memory_dir: String::new(),
            bind_addr: "127.0.0.1:0".into(),
            auth: Some(AuthConfig {
                session_secret: "test-secret-not-a-real-key".into(),
                nc_base_url: "https://nextcloud.example".into(),
                nc_internal_url: None,
                nc_client_id: "id".into(),
                nc_client_secret: "secret".into(),
                nc_redirect_uri: "https://memview.example/auth/callback".into(),
                allowed_users: vec!["user".into()],
            }),
            static_dir: None,
            couse_file: None,
            agents_file: None,
        };
        memview::routes::router(AppState::new(cfg, reqwest::Client::new()))
    }

    /// One GET, with the sign-in cookie if given: the status, where it points,
    /// and the sign-in cookie it sets.
    pub(super) async fn get(
        app: &axum::Router,
        uri: &str,
        signin: Option<&str>,
    ) -> (StatusCode, String, String) {
        let mut req = Request::builder().uri(uri);
        if let Some(value) = signin {
            req = req.header(header::COOKIE, format!("signin={value}"));
        }
        let res = app
            .clone()
            .oneshot(req.body(Body::empty()).unwrap())
            .await
            .expect("the router answered");
        let location = res
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().unwrap().to_string())
            .unwrap_or_default();
        let cookie = res
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok())
            .filter_map(|v| v.strip_prefix("signin="))
            .map(|v| v.split(';').next().unwrap_or("").to_string())
            .find(|v| !v.is_empty())
            .unwrap_or_default();
        (res.status(), location, cookie)
    }

    pub(super) fn state_in(location: &str) -> String {
        url::Url::parse(location)
            .expect("an absolute authorize URL")
            .query_pairs()
            .find(|(k, _)| k == "state")
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    }

    /// ⚠ **The code is dropped, not exchanged.** An exchange would reach
    /// `nextcloud.example` and fail; a redirect to authorize means it never
    /// left. The fresh `state` rides in both the URL and the cookie.
    #[tokio::test]
    async fn a_swallowed_state_is_retried_never_trusted() {
        let app = app();
        let (_, to, first) = get(&app, "/login?return_to=/search", None).await;
        assert_eq!(state_in(&to), first, "login sets what it sends");

        let (status, to, again) =
            get(&app, "/auth/callback?state=&code=attackers", Some(&first)).await;
        assert!(status.is_redirection(), "{status}");
        assert!(to.starts_with("https://nextcloud.example/"), "{to}");
        let fresh = state_in(&to);
        assert_eq!(fresh, again, "the retry's state is the cookie's");
        assert_ne!(fresh, first, "a retry is a new attempt");
    }

    #[tokio::test]
    async fn a_second_miss_is_refused() {
        let app = app();
        let (_, _, first) = get(&app, "/login", None).await;
        let (_, _, again) = get(&app, "/auth/callback?state=&code=a", Some(&first)).await;
        assert!(!again.is_empty(), "the first miss was not retried");
        let (status, to, _) = get(&app, "/auth/callback?state=&code=b", Some(&again)).await;
        assert!(
            !status.is_redirection() && !status.is_success(),
            "{status} {to}"
        );
    }

    /// The retried attempt is spent like any other: its first `state` is no
    /// longer one a callback can name.
    #[tokio::test]
    async fn the_first_attempt_is_spent_by_its_retry() {
        let app = app();
        let (_, _, first) = get(&app, "/login", None).await;
        let (retried, _, _) = get(&app, "/auth/callback?state=&code=a", Some(&first)).await;
        assert!(
            retried.is_redirection(),
            "the miss was not retried: {retried}"
        );
        let (status, _, _) = get(&app, &format!("/auth/callback?state={first}&code=a"), None).await;
        assert!(!status.is_redirection() && !status.is_success(), "{status}");
    }
}

/// The proof `exchange_code` demands is made only by a present `state` equal to
/// the stored one.
#[test]
fn the_proof_is_made_only_by_a_present_matching_state() {
    use memview::nextcloud::identity::StateMatched;
    assert!(StateMatched::check("n0nce", "n0nce").is_some());
    assert!(StateMatched::check("", "").is_none());
    assert!(StateMatched::check("", "n0nce").is_none());
    assert!(StateMatched::check("nonce", "n0nce").is_none());
    assert!(StateMatched::check("n0nc", "n0nce").is_none());
}

/// Login CSRF: someone signs in up to Nextcloud on their own machine, keeps
/// the state and code, and sends a victim's browser to the callback. That
/// browser carries no sign-in cookie, so the state is refused before any code
/// is exchanged.
mod another_browser {
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn a_state_this_browser_never_carried_is_refused() {
        let app = super::retry::app();
        let (_, to, _) = super::retry::get(&app, "/login", None).await;
        let state = super::retry::state_in(&to);
        // No sign-in cookie: the victim's browser never started this sign-in.
        let res = app
            .oneshot(
                Request::get(format!("/auth/callback?state={state}&code=theirs"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("the router answered");
        let body = axum::body::to_bytes(res.into_body(), usize::MAX)
            .await
            .unwrap();
        let body = String::from_utf8_lossy(&body);
        // Refused for the state, before the code went anywhere: an exchange
        // attempt would answer that Nextcloud failed instead.
        assert!(body.contains("did not start in this browser"), "{body}");
    }
}
