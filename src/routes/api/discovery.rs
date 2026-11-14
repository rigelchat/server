use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::rest::discovery::{DiscoveryCategoryResponse, GuildDiscoveryResponse};
use crate::services::guild_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/guilds", get(get_discovery_guilds))
        .route("/guilds/{guild_id}/join", post(join_discovery_guild))
        .route("/categories", get(get_discovery_categories))
}

async fn get_discovery_guilds(
    State(state): State<AppState>,
) -> Result<Json<GuildDiscoveryResponse>, AppError> {
    let res = guild_service::get_discovery_guilds(&state).await?;
    Ok(Json(res))
}

async fn join_discovery_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<StatusCode, AppError> {
    guild_service::add_member_to_guild(&state, &auth.user_id, &guild_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_discovery_categories() -> Json<Vec<DiscoveryCategoryResponse>> {
    Json(vec![])
}