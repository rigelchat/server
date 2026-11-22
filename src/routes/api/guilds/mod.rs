pub mod channels;
pub mod members;
pub mod roles;

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser};
use crate::models::guild::Guild;
use crate::models::rest::guilds::{CreateGuildRequest, UpdateGuildRequest};
use crate::services::guild_service;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_guild))
        .route(
            "/{guild_id}",
            get(get_guild).patch(update_guild).delete(delete_guild),
        )
        .route("/{guild_id}/delete", post(delete_guild))
        .nest("/{guild_id}/roles", roles::router())
        .nest("/{guild_id}/members", members::router())
        .nest("/{guild_id}/channels", channels::router())
}

async fn create_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(payload): AppJson<CreateGuildRequest>,
) -> Result<Json<Guild>, AppError> {
    let guild = guild_service::create_guild(&state, payload, &auth.user_id).await?;
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

async fn update_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
    AppJson(payload): AppJson<UpdateGuildRequest>,
) -> Result<Json<Guild>, AppError> {
    let guild = guild_service::update_guild(&state, &guild_id, &auth.user_id, payload).await?;
    Ok(Json(guild))
}

async fn delete_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<StatusCode, AppError> {
    guild_service::delete_guild(&state, &guild_id, &auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

