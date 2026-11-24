use std::sync::Arc;

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use rigel_server::{
    build_app, config::AppConfig, db, gateway::hub::GatewayHub, state::AppState, utils,
};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let config = Arc::new(AppConfig::from_env());

    utils::assets::spawn_emoji_sync();

    let db = db::init(&config.database_url, config.db_max_connections)
        .await
        .expect("Failed to connect to the database");

    let addr = format!("{}:{}", config.host, config.port);
    let hub = GatewayHub::new();
    hub.spawn_heartbeat_reaper();

    let state = AppState { config, db, hub };
    let app = build_app(state);

    let listener = TcpListener::bind(&addr).await.unwrap();
    info!("API listening at http://{}", listener.local_addr().unwrap());
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .unwrap();
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Shutdown signal received, draining server...");
}
