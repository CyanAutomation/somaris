use std::{env, net::SocketAddr};

use somaris::api::{AppState, router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let address: SocketAddr = env::var("SOMARIS_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_owned())
        .parse()?;
    let state = AppState::from_environment().await?;
    state.spawn_background_tasks();
    let listener = tokio::net::TcpListener::bind(address).await?;
    tracing::info!(%address, "Somaris API listening");
    axum::serve(listener, router(state)).await?;
    Ok(())
}
