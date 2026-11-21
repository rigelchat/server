use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::guild_member::GuildMemberResponse;
use crate::models::rest::guilds::ListMembersQuery;
use crate::services::member_service;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_members))
        .route("/@me", get(get_current_member))
        .route("/{user_id}", get(get_member))
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

async fn get_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((guild_id, user_id)): Path<(String, String)>,
) -> Result<Json<GuildMemberResponse>, AppError> {
    let member =
        member_service::get_guild_member(&state, &guild_id, &auth.user_id, &user_id).await?;
    Ok(Json(member))
}
