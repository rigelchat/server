use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser};
use crate::models::guild_member::GuildMemberResponse;
use crate::models::rest::guilds::{
    ListMembersQuery, UpdateCurrentMemberRequest, UpdateGuildMemberRequest,
};
use crate::services::member_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_members))
        .route("/@me", get(get_current_member).patch(update_current_member))
        .route("/{user_id}", get(get_member).patch(update_member))
        .route(
            "/{user_id}/roles/{role_id}",
            put(add_member_role).delete(remove_member_role),
        )
}

async fn get_members(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
    Query(query): Query<ListMembersQuery>,
) -> Result<Json<Vec<GuildMemberResponse>>, AppError> {
    let members = member_service::get_guild_members(
        &state,
        &guild_id,
        &auth.user_id,
        query.limit,
        query.after.as_deref(),
    )
    .await?;
    Ok(Json(members))
}

async fn get_current_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<Json<GuildMemberResponse>, AppError> {
    let member =
        member_service::get_guild_member(&state, &guild_id, &auth.user_id, &auth.user_id).await?;
    Ok(Json(member))
}

async fn update_current_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
    AppJson(payload): AppJson<UpdateCurrentMemberRequest>,
) -> Result<Json<GuildMemberResponse>, AppError> {
    let member =
        member_service::update_current_member(&state, &guild_id, &auth.user_id, payload).await?;
    Ok(Json(member))
}

async fn get_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((guild_id, user_id)): Path<(String, String)>,
) -> Result<Json<GuildMemberResponse>, AppError> {
    let member =
        member_service::get_guild_member(&state, &guild_id, &auth.user_id, &user_id).await?;
    Ok(Json(member))
}

async fn update_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((guild_id, user_id)): Path<(String, String)>,
    AppJson(payload): AppJson<UpdateGuildMemberRequest>,
) -> Result<Json<GuildMemberResponse>, AppError> {
    let member =
        member_service::update_member(&state, &guild_id, &auth.user_id, &user_id, payload).await?;
    Ok(Json(member))
}

async fn add_member_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((guild_id, user_id, role_id)): Path<(String, String, String)>,
) -> Result<StatusCode, AppError> {
    member_service::add_member_role(&state, &guild_id, &auth.user_id, &user_id, &role_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_member_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((guild_id, user_id, role_id)): Path<(String, String, String)>,
) -> Result<StatusCode, AppError> {
    member_service::remove_member_role(&state, &guild_id, &auth.user_id, &user_id, &role_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
