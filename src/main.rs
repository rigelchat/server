#![recursion_limit = "256"]

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

use axum::Router;
use tower_http::{cors::{Any as CorsAny, CorsLayer}, services::ServeDir, trace::TraceLayer};
use std::{sync::Arc, collections::HashMap, env};
use tokio::{net::TcpListener, sync::RwLock};
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::state::{AppState, AppConfig};

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    utils::assets::spawn_emoji_sync();

    let db = db::init()
        .await
        .expect("Failed to connect to the database");

    let state = AppState {
        config: Arc::new(AppConfig::from_env()),
        db,
        sessions: Arc::new(RwLock::new(HashMap::new()))
    };

    let cors = CorsLayer::new()
        .allow_origin(CorsAny)
        .allow_methods(CorsAny)
        .allow_headers(CorsAny);

    let app = Router::new()
        .nest_service("/cdn", ServeDir::new("static"))
        .nest_service("/static", ServeDir::new("static"))
        .merge(routes::router())
        .route("/gateway", axum::routing::get(gateway::handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let host = env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let addr = format!("{host}:{port}");

    let listener = TcpListener::bind(addr).await.unwrap();
    info!("API listening at http://{}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}