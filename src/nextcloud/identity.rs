//! Nextcloud OAuth2 — identity only.
//!
//! Establishes *who the user is* and nothing else. The access token is used
//! once to read `{id, displayname}` then discarded; no refresh token is
//! stored. Copied from the `messages` app, with recall's public/internal URL
//! split: the *browser* is sent to `nc_base_url`, but the server-side token +
//! userinfo calls can go to `nc_internal_url` (cluster Service DNS) carrying
//! a Host header for `nc_base_url`'s host — on isis a pod can't hairpin to
//! the node's own public IP.

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::config::AuthConfig;

/// Build the URL the browser is redirected to in order to grant access.
pub fn authorize_url(auth: &AuthConfig, state: &str) -> String {
    let mut url = url::Url::parse(&format!(
        "{}/index.php/apps/oauth2/authorize",
        auth.nc_base_url
    ))
    .expect("nc_base_url validated at config load");
    url.query_pairs_mut()
        .append_pair("client_id", &auth.nc_client_id)
        .append_pair("response_type", "code")
        .append_pair("redirect_uri", &auth.nc_redirect_uri)
        .append_pair("state", state);
    url.to_string()
}

/// Server-side base URL + Host header override (None when direct).
fn server_base(auth: &AuthConfig) -> (String, Option<String>) {
    match &auth.nc_internal_url {
        Some(internal) => {
            let host = url::Url::parse(&auth.nc_base_url)
                .ok()
                .and_then(|u| u.host_str().map(|h| h.to_string()));
            (internal.clone(), host)
        }
        None => (auth.nc_base_url.clone(), None),
    }
}

fn with_host(req: reqwest::RequestBuilder, host: &Option<String>) -> reqwest::RequestBuilder {
    match host {
        Some(h) => req.header(reqwest::header::HOST, h.clone()),
        None => req,
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

/// Exchange an authorization `code` for an access token.
pub async fn exchange_code(
    http: &reqwest::Client,
    auth: &AuthConfig,
    code: &str,
) -> Result<String> {
    let (base, host) = server_base(auth);
    let req = http
        .post(format!("{base}/index.php/apps/oauth2/api/v1/token"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", auth.nc_client_id.as_str()),
            ("client_secret", auth.nc_client_secret.as_str()),
            ("redirect_uri", auth.nc_redirect_uri.as_str()),
        ]);
    let res = with_host(req, &host).send().await?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow!("NC token exchange failed: {status}: {body}"));
    }
    let token: TokenResponse = res.json().await.context("parsing NC token response")?;
    Ok(token.access_token)
}

pub struct NcUser {
    pub id: String,
    pub display_name: String,
}

#[derive(Deserialize)]
struct OcsEnvelope {
    ocs: OcsBody,
}
#[derive(Deserialize)]
struct OcsBody {
    data: OcsData,
}
#[derive(Deserialize)]
struct OcsData {
    id: String,
    displayname: String,
}

/// Look up the granting user's id + display name. The token is consumed here.
pub async fn fetch_user(
    http: &reqwest::Client,
    auth: &AuthConfig,
    access_token: &str,
) -> Result<NcUser> {
    let (base, host) = server_base(auth);
    let req = http
        .get(format!("{base}/ocs/v2.php/cloud/user?format=json"))
        .header("Authorization", format!("Bearer {access_token}"))
        .header("OCS-APIRequest", "true");
    let res = with_host(req, &host).send().await?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow!("NC user info failed: {status}: {body}"));
    }
    let parsed: OcsEnvelope = res.json().await.context("parsing NC user info")?;
    Ok(NcUser {
        id: parsed.ocs.data.id,
        display_name: parsed.ocs.data.displayname,
    })
}
