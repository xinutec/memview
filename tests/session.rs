//! Stateless session cookies: a cookie is only accepted when this instance's
//! key signed it and it hasn't expired.

use memview::session::{UserSession, create_session, get_session, sign_value, verify_value};

fn user() -> UserSession {
    UserSession {
        user_id: "pippijn".into(),
        display_name: "Pippijn".into(),
    }
}

#[test]
fn round_trips_the_signed_identity() {
    let cookie = create_session("k", &user());
    let got = get_session("k", &cookie).expect("valid cookie");
    assert_eq!(got.user_id, "pippijn");
    assert_eq!(got.display_name, "Pippijn");
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
