//! The console runner: it owns the Claude Code sessions on this machine.
//!
//!     cargo run -p console
//!
//! Phase 1 of `docs/agent-console.md`: start sessions in allowed directories,
//! read them live, send them instructions. No approvals yet — sessions run in
//! the CLI's default permission mode — and no client authentication, which is
//! why it refuses to listen anywhere but loopback.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use console::api;
use console::config::Config;
use console::roster::Roster;
use tower_http::services::{ServeDir, ServeFile};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "console=info,tower_http=warn".into()),
        )
        .init();

    let config = Config::from_env();
    let address: SocketAddr = config
        .bind
        .parse()
        .with_context(|| format!("BIND_ADDR {:?} is not an address", config.bind))?;
    if !address.ip().is_loopback() {
        // Not a warning and not an override. Until the client-certificate gate
        // exists, listening off loopback means anything that can reach the
        // socket can run code as this user — and the house LAN is full of
        // devices nobody patches. See docs/agent-console.md, *Security model*.
        bail!(
            "refusing to listen on {address}: the console has no client authentication yet, \
             so it may only listen on loopback"
        );
    }

    let static_dir = config.static_dir.clone();
    let dirs = config.dirs.clone();
    let roster = Arc::new(Roster::new(config));
    let mut app = api::router(roster);
    if let Some(dir) = &static_dir {
        // The SPA owns its routes, so anything the API did not answer is the
        // index and not a 404.
        // `fallback`, not `not_found_service`: the latter answers only when the
        // request never reached the directory service, so a deep link like
        // /s/<id> — which is a real path with no file behind it — 404s instead
        // of loading the app.
        app = app.fallback_service(
            ServeDir::new(dir).fallback(ServeFile::new(format!("{dir}/index.html"))),
        );
    }

    let listener = tokio::net::TcpListener::bind(address)
        .await
        .with_context(|| format!("binding {address}"))?;
    tracing::info!(
        "console on http://{address} — sessions allowed in {}",
        dirs.iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    axum::serve(listener, app.into_make_service())
        .await
        .context("serving")?;
    Ok(())
}
