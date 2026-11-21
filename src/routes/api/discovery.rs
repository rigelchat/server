use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::AppState;
use crate::constants::guild::DiscoveryCategory;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::rest::discovery::{
    DiscoveryCategoryResponse, DiscoverySearchQuery, GuildDiscoveryResponse,
};
use crate::services::discovery_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/guilds", get(get_discovery_guilds))
        .route("/guilds/{guild_id}/join", post(join_discovery_guild))
        .route("/categories", get(get_discovery_categories))
}

async fn get_discovery_guilds(
    State(state): State<AppState>,
    Query(params): Query<DiscoverySearchQuery>,
) -> Result<Json<GuildDiscoveryResponse>, AppError> {
    let res = discovery_service::get_discovery_guilds(&state, params).await?;
    Ok(Json(res))
}

async fn join_discovery_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<StatusCode, AppError> {
    discovery_service::join_guild(&state, &auth.user_id, &guild_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_discovery_categories() -> Json<Vec<DiscoveryCategoryResponse>> {
    let categories = DiscoveryCategory::ALL
        .into_iter()
        .map(|cat| DiscoveryCategoryResponse {
            id: cat as u8,
            name: cat.display_name().to_string(),
        })
        .collect();

    Json(categories)
}