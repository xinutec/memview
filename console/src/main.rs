//! The console runner: it owns the Claude Code sessions on this machine.
//!
//!     cargo run -p console
//!
//! It starts sessions in allowed directories, reads them live, sends them
//! instructions and carries their permission questions.
//!
//! Where it listens is the security model: without client authentication it
//! refuses anything but loopback, since whatever reaches the socket runs code as
//! this user. With the TLS gate it serves the world on a pinned client key and
//! keeps a plaintext loopback socket for this machine. See `docs/agent-console.md`.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use console::api;
use console::config::Config;
use console::roster::Roster;
use tower_http::services::ServeDir;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "console=info,tower_http=warn".into()),
        )
        .init();

    // The outbound side is built with no crypto provider baked in, so one must be
    // the process default before any TLS client is built. Already installed is fine.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = Config::from_env();
    let address: SocketAddr = config
        .bind
        .parse()
        .with_context(|| format!("BIND_ADDR {:?} is not an address", config.bind))?;
    if !address.ip().is_loopback() && config.tls.is_none() {
        // Without the client-certificate gate, off-loopback means anything on the LAN
        // can run code as this user. Not a warning.
        bail!(
            "refusing to listen on {address}: no client authentication is configured, \
             so the console may only listen on loopback. Set CONSOLE_TLS_CERT, \
             CONSOLE_TLS_KEY and CONSOLE_CLIENT_KEYS to open it up."
        );
    }
    let gate = match &config.tls {
        Some(tls) => Some(
            console::tls::Gate::new(
                &std::fs::read_to_string(&tls.cert_file)
                    .with_context(|| format!("reading {}", tls.cert_file))?,
                &std::fs::read_to_string(&tls.key_file)
                    .with_context(|| format!("reading {}", tls.key_file))?,
                &tls.pins,
            )?
            .server_config()?,
        ),
        None => None,
    };

    let static_dir = config.static_dir.clone();
    let desk = config.desk.clone();
    let dirs = config.dirs.clone();
    let roster = Arc::new(Roster::new(config));
    // The dashboard is fetched in the background, so no request ever waits on it.
    roster.usage().clone().watch();
    // The sessions are asked a minute apart: the number moves only when a request
    // is answered, and asking puts a line down a live conversation's stdin.
    {
        let asking = roster.clone();
        tokio::spawn(async move {
            loop {
                asking.ask_usage().await;
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        });
    }
    // A sweep that finds nothing costs one comparison per session.
    {
        let watching = roster.clone();
        tokio::spawn(async move {
            loop {
                watching.watch_for_deafness().await;
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
        });
    }
    // Each sweep pays only for conversations whose files have grown.
    {
        let writing = roster.clone();
        tokio::spawn(async move {
            loop {
                writing.write_gists().await;
                tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
            }
        });
    }
    // The only thing here that deletes, so it gets its own line. Hourly.
    {
        let tidying = roster.clone();
        tokio::spawn(async move {
            loop {
                tidying.tidy_images().await;
                tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
            }
        });
    }
    // A `<defunct>` under this console, recorded as it appears. It reaps nothing,
    // so it cannot take an exit status `Session::reap` is waiting for.
    tokio::spawn(console::zombies::watch());
    let carried = roster.inherit();
    if carried > 0 {
        tracing::info!("{carried} session(s) carried across an upgrade — none was restarted");
    }
    // Their kill could not be delivered by the old image: `execve` took the timer.
    let finishing = roster.finish_stopping();
    if finishing > 0 {
        tracing::info!("{finishing} stopped session(s) still to be finished off");
    }
    let mut app = api::router(roster.clone());
    if let Some(dir) = &static_dir {
        // The SPA owns its routes, so a navigation the API did not answer gets the
        // index. `fallback`, not `not_found_service`, which never sees a deep link.
        // Only for a navigation: serving index.html for a missing font once broke the
        // icons with nothing logged anywhere.
        let index = format!("{dir}/index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(axum::routing::any(
            move |uri: axum::http::Uri| {
                let index = index.clone();
                async move { api::spa(&index, uri.path()) }
            },
        )));
    }

    // Take the sessions with us; orphaned ones keep their ids, and `past::in_use`
    // then refuses to resume the very conversations nobody is using.
    {
        let roster = roster.clone();
        tokio::spawn(async move {
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("SIGTERM handler");
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            tracing::info!("stopping — taking the sessions with us");
            roster.shut_down();
            std::process::exit(0);
        });
    }

    // SIGUSR2 upgrades in place, keeping the sessions. A different signal from the
    // one that stops, so `kill` always means stop. See [`Roster::handover`].
    {
        let roster = roster.clone();
        tokio::spawn(async move {
            let mut upgrade =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::user_defined2())
                    .expect("SIGUSR2 handler");
            while upgrade.recv().await.is_some() {
                match roster.handover() {
                    Ok(never) => match never {},
                    Err(error) => tracing::error!("the upgrade did not happen: {error:#}"),
                }
            }
        });
    }

    let where_sessions_run = dirs
        .iter()
        .map(|d| d.display().to_string())
        .collect::<Vec<_>>()
        .join(", ");

    match gate {
        Some(tls_config) => {
            // The desk keeps a way in: the gated socket demands a certificate, and an SSH
            // forward has none to present.
            let desk: SocketAddr = desk
                .parse()
                .with_context(|| format!("CONSOLE_DESK_ADDR {desk:?} is not an address"))?;
            if !desk.ip().is_loopback() {
                bail!("CONSOLE_DESK_ADDR {desk} is not loopback, and it carries no authentication");
            }
            let listener = tokio::net::TcpListener::bind(desk)
                .await
                .with_context(|| format!("binding {desk}"))?;
            tracing::info!(
                "console on https://{address} — client certificate required — \
                 and on http://{desk} for this machine; \
                 sessions allowed in {where_sessions_run}"
            );
            let plain = axum::serve(listener, app.clone().into_make_service());
            let gated = axum_server::bind_rustls(
                address,
                axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls_config)),
            )
            .serve(app.into_make_service());
            // Either socket failing takes the process down rather than leaving a console
            // that is half there.
            tokio::select! {
                served = plain => served.context("serving on loopback")?,
                served = gated => served.context("serving with TLS")?,
            }
        }
        None => {
            let listener = tokio::net::TcpListener::bind(address)
                .await
                .with_context(|| format!("binding {address}"))?;
            tracing::info!(
                "console on http://{address} — sessions allowed in {where_sessions_run}"
            );
            axum::serve(listener, app.into_make_service())
                .await
                .context("serving")?;
        }
    }
    Ok(())
}
