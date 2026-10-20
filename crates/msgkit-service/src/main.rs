use std::net::SocketAddr;
use std::time::Duration;

use msgkit_service::{prune_database, router, AppState, Config};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

/// How often acknowledged event ids past the retention window are forgotten.
const PRUNE_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = Config::try_from_env()?;
    let bind: SocketAddr =
        std::env::var("MSGKIT_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into()).parse()?;

    if !bind.ip().is_loopback() && config.api_token.is_none() {
        warn!("listening on a non-loopback address without MSGKIT_API_TOKEN: /v1/* is unauthenticated");
    }
    if config.limits.unlimited {
        warn!("MSGKIT_NO_LIMITS is set: input limits are switched off, so a single request can use unbounded memory");
    }
    if config.db_path.is_none() {
        warn!("MSGKIT_DB_PATH is not set: webhook events are kept in memory and lost on restart");
    }
    info!(
        whatsapp = config.whatsapp.is_some(),
        messenger = config.messenger.is_some(),
        auth = config.api_token.is_some(),
        durable = config.db_path.is_some(),
        "starting msgkit-service on {bind}"
    );

    if let Some(path) = config.db_path.clone() {
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(PRUNE_INTERVAL);
            tick.tick().await; // the first tick fires immediately; startup already pruned
            loop {
                tick.tick().await;
                let path = path.clone();
                match tokio::task::spawn_blocking(move || prune_database(&path)).await {
                    Ok(Ok(0)) => {}
                    Ok(Ok(n)) => info!("forgot {n} acknowledged event id(s) past retention"),
                    Ok(Err(e)) => error!("database maintenance failed: {e}"),
                    Err(e) => error!("database maintenance task failed: {e}"),
                }
            }
        });
    }

    let state = AppState::new(config)?;
    let listener = tokio::net::TcpListener::bind(bind).await?;
    axum::serve(listener, router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!("shutting down");
        })
        .await?;
    Ok(())
}
