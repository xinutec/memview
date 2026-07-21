//! Request access levels — the health app's owner/share-viewer split.
//!
//! `ReadAccess` admits the owner (session cookie) *or* a share-link
//! recipient (`X-Share-Token` header); `OwnerOnly` admits just the owner —
//! share management must never be reachable through a share token. With
//! auth unconfigured (local dev) every request is the owner.

use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use axum_extra::extract::cookie::CookieJar;

use crate::error::AppError;
use crate::session::{COOKIE_NAME, UserSession, get_session};
use crate::state::AppState;

pub const SHARE_HEADER: &str = "X-Share-Token";

#[derive(Clone, Debug)]
pub enum Viewer {
    Owner(UserSession),
    /// Read-only recipient of the public share link.
    Shared,
}

fn local_owner() -> UserSession {
    UserSession {
        user_id: "local".into(),
        display_name: "Local".into(),
    }
}

fn resolve(app: &AppState, parts: &Parts) -> Result<Viewer, AppError> {
    let Some(auth) = &app.cfg.auth else {
        return Ok(Viewer::Owner(local_owner()));
    };
    let jar = CookieJar::from_headers(&parts.headers);
    if let Some(cookie) = jar.get(COOKIE_NAME)
        && let Some(user) = get_session(&auth.session_secret, cookie.value())
    {
        return Ok(Viewer::Owner(user));
    }
    if let Some(token) = parts
        .headers
        .get(SHARE_HEADER)
        .and_then(|v| v.to_str().ok())
        && app.share.is_valid(token)
    {
        app.share.touch();
        return Ok(Viewer::Shared);
    }
    Err(AppError::Unauthorized)
}

/// Extractor: owner or share-token holder; 401 otherwise.
pub struct ReadAccess(pub Viewer);

impl<S> FromRequestParts<S> for ReadAccess
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        Ok(ReadAccess(resolve(&app, parts)?))
    }
}

/// Extractor: the owner only; a share token gets 403, no auth at all 401.
pub struct OwnerOnly(pub UserSession);

impl<S> FromRequestParts<S> for OwnerOnly
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        match resolve(&app, parts)? {
            Viewer::Owner(user) => Ok(OwnerOnly(user)),
            Viewer::Shared => Err(AppError::Forbidden),
        }
    }
}
