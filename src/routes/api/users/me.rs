use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser, FluxerFeatures};
use crate::models::guild::Guild;
use crate::models::guild_member::GuildMemberResponse;
use crate::models::rest::guilds::ListGuildsQuery;
use crate::models::rest::users::{
    UpdateProfileRequest, UpdateSettingsRequest, UpdateUserRequest, UserProfileResponse,
};
use crate::models::user::{CurrentUser, UserSettings};
use crate::services::{guild, member_service, user};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_current_user).patch(update_user))
        .route("/profile", patch(update_profile))
        .route("/settings", get(get_settings).patch(update_settings))
        .route("/guilds", get(get_guilds))
        .route("/guilds/{guild_id}", delete(leave_guild))
        .route("/guilds/{guild_id}/member", get(get_current_guild_member))
        .route("/mobile-devices", post(no_content))
        .route("/mobile-devices/unregister", post(no_content))
}

async fn get_current_user(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<CurrentUser>, AppError> {
    let user = user::get_current_user(&state, &auth.user_id).await?;
    Ok(Json(user))
}

async fn update_user(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(payload): AppJson<UpdateUserRequest>,
) -> Result<Json<CurrentUser>, AppError> {
    let updated = user::update_user(&state, &auth.user_id, payload).await?;
    Ok(Json(updated))
}

async fn update_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(payload): AppJson<UpdateProfileRequest>,
) -> Result<Json<UserProfileResponse>, AppError> {
    let profile = user::update_profile(&state, &auth.user_id, payload).await?;
    Ok(Json(profile))
}

async fn get_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<UserSettings>, AppError> {
    let settings = user::get_settings(&state, &auth.user_id).await?;
    Ok(Json(settings))
}

async fn update_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    AppJson(payload): AppJson<UpdateSettingsRequest>,
) -> Result<Json<UserSettings>, AppError> {
    let settings = user::update_settings(&state, &auth.user_id, payload).await?;
    Ok(Json(settings))
}

async fn leave_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<StatusCode, AppError> {
    member_service::leave_guild(&state, &auth.user_id, &guild_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_current_guild_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<Json<GuildMemberResponse>, AppError> {
    let member =
        member_service::get_guild_member(&state, &guild_id, &auth.user_id, &auth.user_id).await?;
    Ok(Json(member))
}

async fn no_content() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn get_guilds(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Query(query): Query<ListGuildsQuery>,
) -> Result<Json<Vec<Guild>>, AppError> {
    let guilds = guild::get_user_guilds(
        &state,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        query,
    )
    .await?;
    Ok(Json(guilds))
}
