use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{delete, get, patch, post},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::rest::users::{
    UpdateProfileRequest, UpdateSettingsRequest, UpdateUserRequest, UserProfileResponse,
};
use crate::models::user::{CurrentUser, UserSettings};
use crate::services::{guild_service, user_service};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_current_user).patch(update_user))
        .route("/profile", patch(update_profile))
        .route("/settings", get(get_settings).patch(update_settings))
        .route("/guilds/{guild_id}", delete(leave_guild))
        .route("/mobile-devices", post(no_content))
        .route("/mobile-devices/unregister", post(no_content))
}

async fn get_current_user(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<CurrentUser>, AppError> {
    let user = user_service::get_current_user(&state, &auth.user_id).await?;
    Ok(Json(user))
}

async fn update_user(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpdateUserRequest>,
) -> Result<Json<CurrentUser>, AppError> {
    let updated = user_service::update_user(&state, &auth.user_id, payload).await?;
    Ok(Json(updated))
}

async fn update_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<Json<UserProfileResponse>, AppError> {
    let profile = user_service::update_profile(&state, &auth.user_id, payload).await?;
    Ok(Json(profile))
}

async fn get_settings(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<UserSettings>, AppError> {
    let settings = user_service::get_settings(&state, &auth.user_id).await?;
    Ok(Json(settings))
}

async fn update_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<UpdateSettingsRequest>,
) -> Result<Json<UserSettings>, AppError> {
    let settings = user_service::update_settings(&state, &auth.user_id, payload).await?;
    Ok(Json(settings))
}

async fn leave_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<StatusCode, AppError> {
    guild_service::leave_guild(&state, &auth.user_id, &guild_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn no_content() -> StatusCode {
    StatusCode::NO_CONTENT
}