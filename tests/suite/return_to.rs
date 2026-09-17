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
        // ⚠ The URL Standard treats `\` as `/` for http(s), so a browser reads
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
/// ⚠ This is the security-relevant half of the sign-in and it is a pure
/// function precisely so it can be pinned here: the live flow needs Nextcloud,
/// which no test has, and a decision nobody can exercise is a decision nobody
/// checks.
mod which_signin {
    use memview::routes::auth::state_to_consume;

    #[test]
    fn the_url_is_used_when_the_provider_hands_it_back() {
        // An identity provider that behaves: cookie and URL agree, and either
        // would do. This is the path every other deployment takes.
        assert_eq!(state_to_consume("abc", "abc"), Ok("abc"));
        // No cookie — expired, or cleared between the two requests. The URL
        // still names a pending sign-in, and the in-memory store decides
        // whether it is a real one.
        assert_eq!(state_to_consume("abc", ""), Ok("abc"));
    }

    /// ⚠ **The live case.** Nextcloud is handed 48 hex characters and returns
    /// `state=`, so without this the flow cannot complete at all.
    #[test]
    fn the_cookie_carries_the_signin_when_the_provider_drops_it() {
        assert_eq!(state_to_consume("", "abc"), Ok("abc"));
    }

    /// ⚠ **Refused, not resolved.** A cookie proves this browser began SOME
    /// flow; when the URL names a different one, nothing here can say which is
    /// meant, and picking either would be a guess about an authentication.
    #[test]
    fn two_that_disagree_are_refused_rather_than_guessed() {
        let said = state_to_consume("abc", "xyz").expect_err("must not resolve");
        assert!(said.contains("different sign-in"), "{said:?}");
    }

    #[test]
    fn neither_present_is_refused() {
        let said = state_to_consume("", "").expect_err("must not resolve");
        assert!(said.contains("did not start here"), "{said:?}");
    }

    /// ⚠ Every refusal must be something a person can act on, because it is
    /// rendered as the whole page they are looking at.
    #[test]
    fn every_refusal_is_a_sentence_not_a_code() {
        for (url, cookie) in [("", ""), ("abc", "xyz")] {
            let said = state_to_consume(url, cookie).expect_err("refusal");
            assert!(said.ends_with('.'), "{said:?} should be a sentence");
            assert!(said.len() > 20, "{said:?} is too terse to act on");
        }
    }
}
