mod config;
mod constants;
mod db;
mod error;
mod extractors;
mod gateway;
mod models;
mod routes;
mod services;
mod state;
mod utils;

use std::{collections::HashMap, sync::Arc};

use axum::{Router, http::HeaderValue};
use tokio::{net::TcpListener, sync::RwLock};
use tower_http::{
    cors::{AllowOrigin, Any as CorsAny, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::config::AppConfig;
use crate::state::AppState;

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = Arc::new(AppConfig::from_env());

    if config.features.enable_auto_emoji_sync {
        utils::assets::spawn_emoji_sync();
    }

    let db = db::init(&config.database_url, config.db_max_connections)
        .await
        .expect("Failed to connect to the database");

    let cors = if config.cors_allowed_origins.is_empty() || config.cors_allowed_origins.iter().any(|o| o == "*")
    {
        CorsLayer::new()
            .allow_origin(CorsAny)
            .allow_methods(CorsAny)
            .allow_headers(CorsAny)
    } else {
        let origins: Vec<HeaderValue> = config
            .cors_allowed_origins
            .iter()
            .filter_map(|o| o.parse::<HeaderValue>().ok())
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(CorsAny)
            .allow_headers(CorsAny)
    };

    let addr = format!("{}:{}", config.host, config.port);
    let state = AppState {
        config,
        db,
        sessions: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .nest_service("/static", ServeDir::new("static"))
        .merge(routes::router())
        .route("/gateway", axum::routing::get(gateway::handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = TcpListener::bind(&addr).await.unwrap();
    info!("API listening at http://{}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}