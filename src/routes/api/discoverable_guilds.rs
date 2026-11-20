use axum::{Json, Router, extract::State, routing::get};

use crate::AppState;
use crate::error::AppError;
use crate::models::rest::discoverable_guilds::DiscoverableGuildsResponse;
use crate::services::discovery_service;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_discoverable_guilds))
}

async fn get_discoverable_guilds(
    State(state): State<AppState>,
) -> Result<Json<DiscoverableGuildsResponse>, AppError> {
    let res = discovery_service::get_discoverable_guilds(&state).await?;
    Ok(Json(res))
}