//! Auth routes: Nextcloud identity login, restricted to an explicit
//! allow-list. Copied from `messages`; sessions are stateless here, so
//! logout is just clearing the cookie. All three routes 404 when auth is
//! unconfigured (local dev — there is nothing to log in to).

use anyhow::anyhow;
use axum::extract::{Query, State};
use axum::response::Redirect;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;

use crate::error::AppError;
use crate::nextcloud::identity;
use crate::session::{COOKIE_NAME, UserSession, create_session};
use crate::state::AppState;

fn session_cookie(value: String) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, value))
        .path("/")
        .http_only(true)
        // Not `Secure`: the isis deployment is plain http on the wg0
        // hostPort (the VPN is the transport gate), matching recall.
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(7))
        .build()
}

/// Only allow same-site internal paths as a post-login redirect target.
pub fn validate_return_to(return_to: Option<&str>) -> String {
    match return_to {
        Some(p) if p.starts_with('/') && !p.starts_with("//") => p.to_string(),
        _ => "/".to_string(),
    }
}

#[derive(Deserialize)]
pub struct LoginQuery {
    return_to: Option<String>,
}

/// GET /login → redirect to NC's OAuth2 authorize endpoint.
pub async fn login(
    State(app): State<AppState>,
    Query(q): Query<LoginQuery>,
) -> Result<Redirect, AppError> {
    let auth = app.cfg.auth.as_ref().ok_or(AppError::NotFound)?;
    let state = app.create_oauth_state(q.return_to);
    Ok(Redirect::to(&identity::authorize_url(auth, &state)))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
}

/// GET /auth/callback → exchange code, read identity, ENFORCE the
/// allow-list, then mint our stateless session cookie.
pub async fn callback(
    State(app): State<AppState>,
    jar: CookieJar,
    Query(q): Query<CallbackQuery>,
) -> Result<(CookieJar, Redirect), AppError> {
    let auth = app.cfg.auth.as_ref().ok_or(AppError::NotFound)?;
    let state = q.state.unwrap_or_default();
    let pending = app
        .consume_oauth_state(&state)
        .ok_or(AppError::Unauthorized)?;
    let code = q
        .code
        .ok_or_else(|| anyhow!("missing authorization code"))?;

    let token = identity::exchange_code(&app.http, auth, &code).await?;
    let nc_user = identity::fetch_user(&app.http, auth, &token).await?;

    if !app.cfg.is_allowed(&nc_user.id) {
        tracing::warn!(
            "denied login for non-allowed Nextcloud user {:?}",
            nc_user.id
        );
        return Err(AppError::Forbidden);
    }

    let user = UserSession {
        user_id: nc_user.id,
        display_name: nc_user.display_name,
    };
    let signed = create_session(&auth.session_secret, &user);
    let dest = validate_return_to(pending.return_to.as_deref());
    Ok((jar.add(session_cookie(signed)), Redirect::to(&dest)))
}

/// POST /logout → clear the cookie (sessions are stateless).
pub async fn logout(
    State(app): State<AppState>,
    jar: CookieJar,
) -> Result<(CookieJar, Redirect), AppError> {
    app.cfg.auth.as_ref().ok_or(AppError::NotFound)?;
    Ok((jar.remove(Cookie::from(COOKIE_NAME)), Redirect::to("/")))
}
