use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, patch},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser};
use crate::models::rest::roles::{
    CreateRoleRequest, RoleHoistPositionUpdate, RolePositionUpdate, UpdateRoleRequest,
};
use crate::models::role::Role;
use crate::services::role;

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(get_roles)
                .post(create_role)
                .patch(modify_role_positions),
        )
        .route(
            "/hoist-positions",
            patch(modify_role_hoist_positions).delete(reset_role_hoist_positions),
        )
        .route("/{role_id}", patch(update_role).delete(delete_role))
}

async fn get_roles(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<Json<Vec<Role>>, AppError> {
    let roles = role::get_guild_roles(&state, &guild_id, &auth.user_id).await?;
    Ok(Json(roles))
}

async fn create_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
    AppJson(payload): AppJson<CreateRoleRequest>,
) -> Result<Json<Role>, AppError> {
    let role = role::create_role(&state, &guild_id, &auth.user_id, payload).await?;
    Ok(Json(role))
}

async fn update_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((guild_id, role_id)): Path<(String, String)>,
    AppJson(payload): AppJson<UpdateRoleRequest>,
) -> Result<Json<Role>, AppError> {
    let role = role::update_role(&state, &guild_id, &role_id, &auth.user_id, payload).await?;
    Ok(Json(role))
}

async fn delete_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((guild_id, role_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    role::delete_role(&state, &guild_id, &role_id, &auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn modify_role_positions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
    AppJson(payload): AppJson<Vec<RolePositionUpdate>>,
) -> Result<StatusCode, AppError> {
    role::modify_role_positions(&state, &guild_id, &auth.user_id, payload).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn modify_role_hoist_positions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
    AppJson(payload): AppJson<Vec<RoleHoistPositionUpdate>>,
) -> Result<StatusCode, AppError> {
    role::modify_role_hoist_positions(&state, &guild_id, &auth.user_id, payload).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_role_hoist_positions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<StatusCode, AppError> {
    role::reset_role_hoist_positions(&state, &guild_id, &auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
