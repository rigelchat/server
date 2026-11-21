pub mod config;
pub mod constants;
pub mod db;
pub mod error;
pub mod extractors;
pub mod gateway;
pub mod models;
pub mod routes;
pub mod services;
pub mod state;
pub mod utils;

use axum::{Router, http::HeaderValue};
use tower_http::{
    cors::{AllowOrigin, Any as CorsAny, CorsLayer},
    services::ServeDir,
    trace::TraceLayer,
};

use crate::state::AppState;

pub fn build_app(state: AppState) -> Router {
    let cors = if state.config.cors_allowed_origins.is_empty()
        || state.config.cors_allowed_origins.iter().any(|o| o == "*")
    {
        CorsLayer::new()
            .allow_origin(CorsAny)
            .allow_methods(CorsAny)
            .allow_headers(CorsAny)
    } else {
        let origins: Vec<HeaderValue> = state
            .config
            .cors_allowed_origins
            .iter()
            .filter_map(|o| match o.parse::<HeaderValue>() {
                Ok(val) => Some(val),
                Err(err) => {
                    tracing::warn!(origin = %o, error = %err, "Ignoring invalid CORS origin");
                    None
                }
            })
            .collect();
        CorsLayer::new()
            .allow_origin(AllowOrigin::list(origins))
            .allow_methods(CorsAny)
            .allow_headers(CorsAny)
    };

    Router::new()
        .nest_service("/static", ServeDir::new("static"))
        .merge(routes::router())
        .route("/gateway", axum::routing::get(gateway::handler))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}