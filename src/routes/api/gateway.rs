use axum::{Json, Router, extract::State, routing::get};

use crate::AppState;
use crate::models::rest::gateway::{GatewayBotResponse, GatewayResponse, SessionStartLimit};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(gateway))
        .route("/bot", get(bot_gateway))
}

async fn gateway(State(state): State<AppState>) -> Json<GatewayResponse> {
    Json(GatewayResponse {
        url: format!("{}/gateway", state.config.public_ws_url),
    })
}

async fn bot_gateway(State(state): State<AppState>) -> Json<GatewayBotResponse> {
    Json(GatewayBotResponse {
        url: format!("{}/gateway", state.config.public_ws_url),
        shards: 1,
        session_start_limit: SessionStartLimit {
            total: 1000,
            remaining: 999,
            max_concurrency: 1,
            reset_after: 14400000,
        },
    })
}
