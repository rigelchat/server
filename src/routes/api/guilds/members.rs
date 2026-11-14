use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::guild_member::GuildMemberResponse;
use crate::services::guild_service;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_members))
}

async fn get_members(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<Json<Vec<GuildMemberResponse>>, AppError> {
    let members = guild_service::get_guild_members(&state, &guild_id, &auth.user_id).await?;
    Ok(Json(members))
}