//! The console runner: it owns the Claude Code sessions on this machine.
//!
//!     cargo run -p console
//!
//! It starts sessions in allowed directories, reads them live, sends them
//! instructions and carries their permission questions.
//!
//! **Where it listens is the security model, not a setting.** With no client
//! authentication configured it refuses to listen anywhere but loopback, because
//! anything that can reach the socket can run code as this user and the house LAN
//! is full of devices nobody patches. With the gate configured it serves the
//! world on TLS with a pinned client key — and keeps a plaintext loopback socket
//! beside it for this machine, which is sound for the same reason the loopback-only
//! mode is: a local process can spawn `claude` itself. See `docs/agent-console.md`.

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

    // ⚠ **Before any TLS client is built.** The outbound side is compiled with
    // no crypto provider baked in — see `Cargo.toml` for why aws-lc is not
    // wanted here — so one has to be the process default or building a client
    // fails at run time rather than at compile time. Ring, the same provider the
    // listener below is built with explicitly. Already-installed is not an
    // error worth reporting: it means somebody got here first.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = Config::from_env();
    let address: SocketAddr = config
        .bind
        .parse()
        .with_context(|| format!("BIND_ADDR {:?} is not an address", config.bind))?;
    if !address.ip().is_loopback() && config.tls.is_none() {
        // Not a warning and not an override. Without the client-certificate
        // gate, listening off loopback means anything that can reach the socket
        // can run code as this user — and the house LAN is full of devices
        // nobody patches. See docs/agent-console.md, *Security model*.
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
    // Before anything else: if this image was exec'd by an upgrade, the sessions
    // it was running are still running and their pipes came with us.
    // Fetched in the background from here on, so no request ever waits on the
    // dashboard and a console with none configured simply never asks.
    roster.usage().clone().watch();
    // And asked of the sessions themselves, which is where the current figures
    // are. A minute apart: the number moves only when a request is answered, and
    // this puts a line down a live conversation's stdin to get it.
    {
        let asking = roster.clone();
        tokio::spawn(async move {
            loop {
                asking.ask_usage().await;
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            }
        });
    }
    // Sessions that have stopped reading what is written to them. Fifteen
    // seconds apart: a sweep that finds nothing costs one comparison per
    // session, and the number that matters is how long somebody stares at a
    // *waiting to be read* marker before the console admits what it means.
    {
        let watching = roster.clone();
        tokio::spawn(async move {
            loop {
                watching.watch_for_deafness().await;
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            }
        });
    }
    // What each conversation is about, written by the cheapest model there is
    // from the transcript itself — see [`console::gist`]. A quarter of an hour
    // apart, and each sweep pays only for the conversations whose files have
    // grown since their last sentence, so an idle console spends nothing.
    {
        let writing = roster.clone();
        tokio::spawn(async move {
            loop {
                writing.write_gists().await;
                tokio::time::sleep(std::time::Duration::from_secs(15 * 60)).await;
            }
        });
    }
    // The pictures the phone has sent, kept only as long as the conversations
    // they belong to — see [`console::images::tidy`]. Its own loop rather than a
    // second job inside the one above, because it is the only thing here that
    // deletes and that deserves to be visible on its own line. Hourly: what it
    // reclaims is one directory per conversation deleted, which is not something
    // that happens on a timescale worth chasing.
    {
        let tidying = roster.clone();
        tokio::spawn(async move {
            loop {
                tidying.tidy_images().await;
                tokio::time::sleep(std::time::Duration::from_secs(60 * 60)).await;
            }
        });
    }
    // A `<defunct>` under this console, recorded when it appears rather than
    // counted long afterwards — see [`console::zombies`] and #797. It reads the
    // process table and reaps nothing, so it cannot take an exit status
    // `Session::reap` is waiting for.
    tokio::spawn(console::zombies::watch());
    let carried = roster.inherit();
    if carried > 0 {
        tracing::info!("{carried} session(s) carried across an upgrade — none was restarted");
    }
    // And the ones the old image was in the middle of stopping, whose kill it
    // could not deliver because `execve` took the timer with it. See #750.
    let finishing = roster.finish_stopping();
    if finishing > 0 {
        tracing::info!("{finishing} stopped session(s) still to be finished off");
    }
    let mut app = api::router(roster.clone());
    if let Some(dir) = &static_dir {
        // The SPA owns its routes, so anything the API did not answer is the
        // index and not a 404.
        // `fallback`, not `not_found_service`: the latter answers only when the
        // request never reached the directory service, so a deep link like
        // /s/<id> — which is a real path with no file behind it — 404s instead
        // of loading the app.
        //
        // ⚠ **But only for a navigation.** Falling back for *everything* meant a
        // file that was briefly missing — the bundle is rewritten in place on
        // every build — came back as `200 text/html`, and a browser handed HTML
        // where it asked for a font neither retries nor complains. The icons
        // vanished on a reload and nothing anywhere recorded a failure: not the
        // server log, not the client trace, not the network panel. A 404 is the
        // answer that can be seen.
        let index = format!("{dir}/index.html");
        app = app.fallback_service(ServeDir::new(dir).fallback(axum::routing::any(
            move |uri: axum::http::Uri| {
                let index = index.clone();
                async move { api::spa(&index, uri.path()) }
            },
        )));
    }

    // Take the sessions with us. Without this, stopping the console orphans every
    // `claude` it started: they keep running, keep their session ids, and keep
    // appearing in the process table, where `past::in_use` reads them and refuses
    // to resume the very conversations nobody is using any more.
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

    // SIGUSR2 upgrades in place, keeping the sessions. Deliberately a different
    // signal from the one that stops: `kill` means stop, and an upgrade that
    // answered to it would be a stop that sometimes did not stop. nginx spells
    // it the same way, for the same reason.
    //
    // If it fails, the console carries on as it was — see [`Roster::handover`]
    // for why returning beats exiting.
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
            // The desk keeps a way in. Without this, turning the gate on takes the
            // console away from the machine it runs on: the gated socket demands a
            // certificate of everybody, and an SSH forward has none to present.
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
            // Either one falling over takes the process down rather than leaving a
            // console that is half there — which from a phone looks exactly like
            // the Mac being asleep.
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
