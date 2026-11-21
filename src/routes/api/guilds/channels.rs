use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::channel::ChannelResponse;
use crate::services::channel_service;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_guild_channels))
}

async fn get_guild_channels(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
) -> Result<Json<Vec<ChannelResponse>>, AppError> {
    let channels = channel_service::get_guild_channels(&state, &guild_id, &auth.user_id).await?;
    Ok(Json(channels))
}
