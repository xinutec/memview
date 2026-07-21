//! memview — read-only viewer for the Claude memory corpus. Loads config,
//! loads the share-token state file, serves. All logic lives in the
//! `memview` library crate.

use anyhow::Result;
use memview::{config::Config, routes, share::ShareStore, state::AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = Config::from_env()?;
    match &cfg.auth {
        Some(auth) => {
            if auth.allowed_users.is_empty() {
                anyhow::bail!("ALLOWED_USERS is empty — refusing to start (would deny everyone)");
            }
            tracing::info!("auth enabled; allow-list: {:?}", auth.allowed_users);
        }
        None => tracing::warn!("auth NOT configured — serving open (dev mode)"),
    }
    // Fail fast on an unreadable corpus rather than 500ing every request.
    let corpus = memview::store::Corpus::load(&cfg.memory_dir)?;
    tracing::info!(
        "corpus: {} memories in {}",
        corpus.docs.len(),
        cfg.memory_dir
    );

    let share = ShareStore::load(&cfg.share_state_file)?;
    let http = reqwest::Client::builder().build()?;
    let bind_addr = cfg.bind_addr.clone();
    let app = routes::router(AppState::new(cfg, http, share));

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    tracing::info!("memview listening on {bind_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
