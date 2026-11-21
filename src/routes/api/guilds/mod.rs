pub mod channels;
pub mod members;

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser};
use crate::models::guild::Guild;
use crate::models::rest::guilds::CreateGuildRequest;
use crate::models::role::Role;
use crate::services::{guild_service, role_service};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_guild))
        .route("/{guild_id}", get(get_guild))
        .route("/{guild_id}/roles", get(get_roles))
        .nest("/{guild_id}/members", members::router())
        .nest("/{guild_id}/channels", channels::router())
}

async fn create_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(payload): AppJson<CreateGuildRequest>,
) -> Result<Json<Guild>, AppError> {
    let guild = guild_service::create_guild(&state, &payload.name, &auth.user_id).await?;
    Ok(Json(guild))
}

async fn get_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<Json<Guild>, AppError> {
    let guild = guild_service::get_guild(&state, &guild_id, &auth.user_id).await?;
    Ok(Json(guild))
}

async fn get_roles(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<Json<Vec<Role>>, AppError> {
    let roles = role_service::get_guild_roles(&state, &guild_id, &auth.user_id).await?;
    Ok(Json(roles))
}
