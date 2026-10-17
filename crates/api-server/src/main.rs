//! `shado-engine` binary (docs/10 §6 boot order): config → tracing →
//! transport selection (sqlx pool if `DATABASE_URL`, else in-memory) →
//! router serve.

use std::sync::Arc;

use api_server::config::Config;
use api_server::db;
use api_server::repo::Repos;
use api_server::routes::build_router;
use api_server::state::AppState;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = Config::from_env().map_err(|e| {
        eprintln!("invalid config: {e}");
        e
    })?;
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();
    tracing::info!(
        bind = %cfg.bind,
        pg = cfg.database_url.is_some(),
        "shado-engine starting (docs/10 §6)"
    );

    let repos: Arc<dyn Repos> = db::build_repos(&cfg.database_url).await;
    let state = Arc::new(AppState::new(cfg.clone(), repos));
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(cfg.bind).await.map_err(|e| {
        eprintln!("bind {}: {e}", cfg.bind);
        e
    })?;
    tracing::info!(addr = %cfg.bind, "listening");
    if let Err(e) = axum::serve(listener, app).await {
        return Err(e.into());
    }
    Ok(())
}
