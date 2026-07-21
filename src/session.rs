//! Stateless HMAC-signed sessions (the recall pattern — no DB in this app).
//!
//! The cookie carries its own claims: `base64url(json{u,d,x}).hex(hmac)`.
//! Nextcloud is touched only at login; every later request verifies the
//! signature and expiry locally. Survives restarts and a read-only rootfs.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Duration, Utc};
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

const SESSION_TTL_DAYS: i64 = 7;
pub const COOKIE_NAME: &str = "session";

#[derive(Clone, Debug)]
pub struct UserSession {
    pub user_id: String,
    pub display_name: String,
}

#[derive(Serialize, Deserialize)]
struct Claims {
    u: String,
    d: String,
    /// Expiry, unix seconds.
    x: i64,
}

pub fn sign_value(secret: &str, value: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac accepts any key length");
    mac.update(value.as_bytes());
    format!("{value}.{}", hex::encode(mac.finalize().into_bytes()))
}

/// Verify a signed value, returning the inner payload (None if bad).
/// Constant-time on the signature comparison.
pub fn verify_value(secret: &str, signed: &str) -> Option<String> {
    let idx = signed.rfind('.')?;
    let (value, dotted_sig) = signed.split_at(idx);
    let sig = hex::decode(&dotted_sig[1..]).ok()?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).ok()?;
    mac.update(value.as_bytes());
    mac.verify_slice(&sig).ok()?;
    Some(value.to_string())
}

/// Mint a signed cookie value for this user, valid SESSION_TTL_DAYS.
pub fn create_session(secret: &str, user: &UserSession) -> String {
    let claims = Claims {
        u: user.user_id.clone(),
        d: user.display_name.clone(),
        x: (Utc::now() + Duration::days(SESSION_TTL_DAYS)).timestamp(),
    };
    let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).expect("claims serialize"));
    sign_value(secret, &payload)
}

/// Resolve a signed cookie back to a session (None if forged or expired).
pub fn get_session(secret: &str, signed: &str) -> Option<UserSession> {
    let payload = verify_value(secret, signed)?;
    let claims: Claims = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()?;
    if claims.x < Utc::now().timestamp() {
        return None;
    }
    Some(UserSession {
        user_id: claims.u,
        display_name: claims.d,
    })
}
