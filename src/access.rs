//! Who may read. The owner, signed in with a session cookie, and nobody else;
//! with auth unconfigured (local dev) every request is the owner.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum_extra::extract::cookie::CookieJar;

use crate::error::AppError;
use crate::session::{COOKIE_NAME, UserSession, get_session};
use crate::state::AppState;

/// Extractor: the signed-in owner; 401 otherwise.
pub struct Owner(pub UserSession);

impl<S> FromRequestParts<S> for Owner
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let Some(auth) = &app.cfg.auth else {
            return Ok(Owner(UserSession {
                user_id: "local".into(),
                display_name: "Local".into(),
            }));
        };
        let jar = CookieJar::from_headers(&parts.headers);
        jar.get(COOKIE_NAME)
            .and_then(|cookie| get_session(&auth.session_secret, cookie.value()))
            .map(Owner)
            .ok_or(AppError::Unauthorized)
    }
}
