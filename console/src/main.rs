//! The console runner: it owns the Claude Code sessions on this machine.
//!
//!     cargo run -p console
//!
//! Loopback only without the TLS gate, since whatever reaches the socket runs code
//! as this user; with it, pinned client keys plus a loopback socket.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use console::api;
use console::config::Config;
use console::roster::Roster;

/// The agent console: the service, or with `hook`, Claude Code's hook for
/// `Bash`.
#[derive(Parser)]
struct Cli {
    /// The service when absent.
    #[command(subcommand)]
    verb: Option<Verb>,
}

#[derive(Subcommand)]
enum Verb {
    /// Claude Code's hook for `Bash`, run by every session on this Mac.
    Hook,
    /// Run a command as root: the console asks for the password on the phone and
    /// runs it, so the session never sees the password.
    Sudo {
        /// The command, run by `bash -c` in the current directory.
        command: String,
    },
    /// Print the address isis listens on for the phone, which the console's
    /// certificate must name.
    WayIn,
}

#[tokio::main]
async fn main() -> Result<()> {
    // `agent-console hook`: Claude Code's hook for `Bash`, run by every session on
    // this Mac. See [`hook`].
    let verb = Cli::parse().verb;
    if let Some(Verb::WayIn) = verb {
        println!(
            "{}",
            console::tunnel::Dial::from_env("", std::path::Path::new("")).listen
        );
        return Ok(());
    }
    if let Some(Verb::Sudo { command }) = verb {
        let _ = rustls::crypto::ring::default_provider().install_default();
        match sudo(command).await {
            Ok(ran) => {
                print!("{}", ran.stdout);
                eprint!("{}", ran.stderr);
                std::process::exit(ran.status);
            }
            Err(why) => {
                eprintln!("the agent console could not be reached: {why:#}");
                std::process::exit(1);
            }
        }
    }
    if let Some(Verb::Hook) = verb {
        let _ = rustls::crypto::ring::default_provider().install_default();
        if let Err(why) = hook().await {
            // Exit 2 blocks the call and tells the session why.
            eprintln!("the agent console could not be reached: {why:#}");
            std::process::exit(2);
        }
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "console=info,tower_http=warn".into()),
        )
        .init();

    // A crypto provider before any TLS client is built.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let config = Config::from_env();
    let address: SocketAddr = config
        .bind
        .parse()
        .with_context(|| format!("BIND_ADDR {:?} is not an address", config.bind))?;
    if !address.ip().is_loopback() && config.tls.is_none() {
        // Off-loopback without the gate: anything on the LAN could run code here.
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
    // A minute apart: each ask writes to a live session's stdin.
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
    // Records any `<defunct>` child; reaps nothing.
    tokio::spawn(console::zombies::watch());
    roster.open_way_in();
    let carried = roster.inherit();
    if carried > 0 {
        tracing::info!("{carried} session(s) carried across an upgrade — none was restarted");
    }
    // Kills the old image could not deliver.
    let finishing = roster.finish_stopping();
    if finishing > 0 {
        tracing::info!("{finishing} stopped session(s) still to be finished off");
    }
    let mut app = api::router(roster.clone());
    if let Some(dir) = &static_dir {
        app = app.fallback_service(console::serving::app_files(dir));
    }

    // Take the sessions with us, or orphans block their own resume.
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

    // SIGUSR2 upgrades in place — [`Roster::handover`]; `kill` always means stop.
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
            // For an SSH forward, which has no certificate.
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
            // Either socket failing takes the process down.
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

/// How long the hook keeps trying, across an upgrade's moment of closed port.
const HOOK_PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

/// Hand the hook's input to the console and wait for it to answer.
async fn hook() -> Result<()> {
    use std::io::Read as _;
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input)?;
    let url = format!("http://{}/api/hook", Config::from_env().desk);
    let client = reqwest::Client::new();
    let deadline = std::time::Instant::now() + HOOK_PATIENCE;
    loop {
        let sent = client
            .post(&url)
            .header("content-type", "application/json")
            .body(input.clone())
            .send()
            .await;
        match sent {
            Ok(answer) => {
                answer
                    .error_for_status()
                    .with_context(|| format!("at {url}"))?;
                return Ok(());
            }
            Err(why) if why.is_connect() && std::time::Instant::now() < deadline => {
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
            Err(why) => return Err(why).with_context(|| format!("at {url}")),
        }
    }
}

/// Ask the console to run `command` as root, and wait for the person to answer.
async fn sudo(command: String) -> Result<console::sudo::Ran> {
    let asked = console::sudo::Request {
        session: std::env::var("CLAUDE_CODE_SESSION_ID")
            .context("not run by a Claude Code session")?,
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        command,
    };
    let url = format!("http://{}/api/sudo", Config::from_env().desk);
    let ran = reqwest::Client::new()
        .post(&url)
        .json(&asked)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .with_context(|| format!("at {url}"))?
        .json()
        .await?;
    Ok(ran)
}
