use axum::{Json, Router, extract::State, routing::get};

use crate::AppState;
use crate::constants::{
    API_VERSION,
    gateway::{GatewayCompression, GatewayEncoding},
};
use crate::models::rest::well_known::spacebar;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_config))
}

async fn get_config(State(state): State<AppState>) -> Json<spacebar::ConfigResponse> {
    Json(spacebar::ConfigResponse {
        api: spacebar::Api {
            base_url: state.config.public_http_url.clone(),
            api_versions: spacebar::ApiVersions {
                default: API_VERSION,
                active: vec![API_VERSION],
            },
        },
        cdn: spacebar::Cdn {
            base_url: format!("{}/cdn", state.config.public_http_url),
        },
        gateway: spacebar::Gateway {
            base_url: format!("{}/gateway", state.config.public_ws_url),
            encoding: vec![GatewayEncoding::Json],
            compression: vec![Some(GatewayCompression::ZstdStream), None],
        },
    })
}
