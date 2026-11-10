//! Stateless session cookies: a cookie is only accepted when this instance's
//! key signed it and it hasn't expired.

use memview::session::{
    UserSession, create_session, get_session, renewed, sign_value, verify_value,
};

fn user() -> UserSession {
    UserSession {
        user_id: "user".into(),
        display_name: "User".into(),
    }
}

#[test]
fn round_trips_the_signed_identity() {
    let cookie = create_session("k", &user());
    let got = get_session("k", &cookie).expect("valid cookie");
    assert_eq!(got.user_id, "user");
    assert_eq!(got.display_name, "User");
}

#[test]
fn rejects_a_cookie_signed_with_another_key() {
    let cookie = create_session("k", &user());
    assert!(get_session("other", &cookie).is_none());
}

#[test]
fn rejects_a_tampered_payload() {
    let cookie = create_session("k", &user());
    // Flip the first payload byte, keeping the original signature.
    let forged = format!("A{}", &cookie[1..]);
    assert!(get_session("k", &forged).is_none());
}

#[test]
fn rejects_a_forged_claim_set() {
    // Re-signing a payload with the wrong key must not authenticate — the
    // attacker controls the claims but not the key.
    let forged = sign_value(
        "attacker-key",
        "eyJ1IjoicGlwcGlqbiIsImQiOiJQIiwieCI6OTk5OTk5OTk5OX0",
    );
    assert!(get_session("k", &forged).is_none());
}

#[test]
fn rejects_malformed_cookies() {
    for bad in ["", "no-dot", "a.b", "."] {
        assert!(get_session("k", bad).is_none(), "accepted {bad:?}");
    }
}

#[test]
fn signature_helpers_agree() {
    let signed = sign_value("k", "payload");
    assert_eq!(verify_value("k", &signed).as_deref(), Some("payload"));
    assert!(verify_value("other", &signed).is_none());
}

/// A cookie expiring at `x` (unix seconds), signed with `key`.
fn expiring_at(key: &str, x: i64) -> String {
    use base64::Engine;
    let claims = format!(r#"{{"u":"user","d":"User","x":{x}}}"#);
    sign_value(
        key,
        &base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(claims),
    )
}

#[test]
fn a_fresh_cookie_is_not_reminted() {
    assert!(renewed("k", &create_session("k", &user())).is_none());
}

#[test]
fn a_used_cookie_is_reminted_for_a_month_from_now() {
    let now = chrono::Utc::now().timestamp();
    let ten_days = expiring_at("k", now + 10 * 86_400);
    let fresh = renewed("k", &ten_days).expect("a cookie ten days from expiry is renewed");
    let got = get_session("k", &fresh).expect("the renewed cookie is valid");
    assert_eq!(got.user_id, "user");
    assert!(renewed("k", &fresh).is_none(), "and is fresh itself");
}

#[test]
fn an_expired_or_forged_cookie_is_never_reminted() {
    let now = chrono::Utc::now().timestamp();
    assert!(renewed("k", &expiring_at("k", now - 1)).is_none());
    assert!(renewed("k", &expiring_at("other", now + 86_400)).is_none());
}
