//! Auth routes: Nextcloud identity login, restricted to an allow-list. Sessions
//! are stateless, so logout is clearing the cookie. All three routes 404 when
//! auth is unconfigured.

use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;

use crate::error::AppError;
use crate::nextcloud::identity;
use crate::session::{COOKIE_NAME, UserSession, create_session};
use crate::state::{AppState, OAUTH_TTL};

/// Names the sign-in this browser started, so the callback can be matched to it
/// even when the identity provider loses the `state` it was given.
const PENDING_COOKIE: &str = "signin";

fn session_cookie(value: String) -> Cookie<'static> {
    Cookie::build((COOKIE_NAME, value))
        .path("/")
        .http_only(true)
        // Not `Secure`: the isis deployment is plain http on the wg0 hostPort.
        .same_site(SameSite::Lax)
        .max_age(time::Duration::days(7))
        .build()
}

/// Only same-site internal paths as a post-login redirect target. The test is
/// on the character AFTER the leading slash: `/\host` is protocol-relative too,
/// since the URL Standard treats a backslash as a slash for http(s). A backslash
/// anywhere LATER is an ordinary path character.
pub fn validate_return_to(return_to: Option<&str>) -> String {
    match return_to {
        Some(p) if p.starts_with('/') && !matches!(p.as_bytes().get(1), Some(b'/' | b'\\')) => {
            p.to_string()
        }
        _ => "/".to_string(),
    }
}

#[derive(Deserialize)]
pub struct LoginQuery {
    return_to: Option<String>,
}

/// The cookie that says this browser started a sign-in. `Lax`, and it must not
/// be `Strict`: the callback is a top-level cross-site navigation, on which a
/// `Strict` cookie is withheld.
fn pending_cookie(state: String) -> Cookie<'static> {
    Cookie::build((PENDING_COOKIE, state))
        .path("/")
        .http_only(true)
        // Not `Secure`, for the reason `session_cookie` gives.
        .same_site(SameSite::Lax)
        .max_age(
            time::Duration::try_from(OAUTH_TTL).unwrap_or_else(|_| time::Duration::seconds(600)),
        )
        .build()
}

/// GET /login → redirect to NC's OAuth2 authorize endpoint.
pub async fn login(
    State(app): State<AppState>,
    jar: CookieJar,
    Query(q): Query<LoginQuery>,
) -> Result<(CookieJar, Redirect), AppError> {
    let auth = app.cfg.auth.as_ref().ok_or(AppError::NotFound)?;
    let state = app.create_oauth_state(q.return_to);
    // The same value goes two ways — the URL, and a cookie only this browser can
    // hold. See `state_to_consume`.
    Ok((
        jar.add(pending_cookie(state.clone())),
        Redirect::to(&identity::authorize_url(auth, &state)),
    ))
}

/// Which pending sign-in this callback is for: the one named in the URL, or —
/// when the provider dropped it — the one this browser is carrying.
///
/// Nextcloud loses the `state` it was given: it stashes it in its PHP session
/// (`LoginRedirectorController.php:95`), and a sign-in that crosses its own
/// login page comes back as `state=&code=…`; its `__Host-nc_sameSiteCookiestrict`
/// is withheld on the cross-site redirect in, so it demands a login even when
/// signed in. An `HttpOnly` cookie proves the flow is this browser's just as
/// well; what it cannot say is WHICH flow, so two present and disagreeing is
/// refused.
pub fn state_to_consume<'a>(
    from_url: &'a str,
    from_cookie: &'a str,
) -> Result<&'a str, &'static str> {
    match (from_url, from_cookie) {
        ("", "") => Err("This sign-in did not start here, or it took too long."),
        (url, cookie) if !url.is_empty() && !cookie.is_empty() && url != cookie => {
            Err("This link answers a different sign-in attempt.")
        }
        ("", cookie) => Ok(cookie),
        (url, _) => Ok(url),
    }
}

/// A sign-in that could not be finished, drawn for the BROWSER: `/auth/callback`
/// is somewhere a browser is sent, and `AppError`'s JSON on a phone reads as the
/// application being broken.
fn sign_in_problem(status: StatusCode, said: &str) -> Response {
    let said = said
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let body = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>Sign-in did not finish</title><style>\
         body{{font:16px/1.5 system-ui,-apple-system,sans-serif;margin:0;\
         min-height:100vh;display:grid;place-items:center;padding:1.5rem;color:#1a1a1a}}\
         main{{max-width:26rem}}h1{{font-size:1.2rem;margin:0 0 .5rem}}\
         p{{margin:0 0 1.5rem;color:#555}}\
         a{{display:inline-block;padding:.65rem 1.1rem;border-radius:.5rem;\
         background:#1b6ac9;color:#fff;text-decoration:none}}\
         </style></head><body><main><h1>Sign-in did not finish</h1>\
         <p>{said}</p><a href=\"/login\">Try again</a></main></body></html>"
    );
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
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
) -> Response {
    match finish_sign_in(app, jar, q).await {
        Ok(done) => done.into_response(),
        // A failure here is HTML, not `AppError`'s JSON — see `sign_in_problem`.
        Err(said) => sign_in_problem(StatusCode::UNAUTHORIZED, said).into_response(),
    }
}

/// The sign-in itself, split out so every failure has one place to become a page.
async fn finish_sign_in(
    app: AppState,
    jar: CookieJar,
    q: CallbackQuery,
) -> Result<(CookieJar, Redirect), &'static str> {
    let auth = app
        .cfg
        .auth
        .as_ref()
        .ok_or("Sign-in is not configured on this server.")?;

    // Cleared whatever happens: a pending sign-in is spent by being answered.
    let from_cookie = jar
        .get(PENDING_COOKIE)
        .map(|c| c.value().to_string())
        .unwrap_or_default();
    let jar = jar.remove(Cookie::from(PENDING_COOKIE));

    let from_url = q.state.unwrap_or_default();
    let state = state_to_consume(&from_url, &from_cookie)?;
    let pending = app
        .consume_oauth_state(state)
        .ok_or("This sign-in did not start here, or it took too long.")?;
    let code = q
        .code
        .ok_or("The identity provider sent no authorization code.")?;

    let token = identity::exchange_code(&app.http, auth, &code)
        .await
        .map_err(|e| {
            tracing::error!("code exchange failed: {e:#}");
            "Could not complete the sign-in with Nextcloud."
        })?;
    let nc_user = identity::fetch_user(&app.http, auth, &token)
        .await
        .map_err(|e| {
            tracing::error!("identity lookup failed: {e:#}");
            "Could not read your Nextcloud account."
        })?;

    if !app.cfg.is_allowed(&nc_user.id) {
        tracing::warn!(
            "denied login for non-allowed Nextcloud user {:?}",
            nc_user.id
        );
        return Err("That account is not allowed to sign in here.");
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
