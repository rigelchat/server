use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser, FluxerFeatures};
use crate::models::channel::ChannelResponse;
use crate::models::rest::channels::{ChannelPositionUpdate, CreateChannelRequest};
use crate::services::channel;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/",
        get(get_guild_channels)
            .post(create_guild_channel)
            .patch(modify_channel_positions),
    )
}

async fn get_guild_channels(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(guild_id): Path<String>,
) -> Result<Json<Vec<ChannelResponse>>, AppError> {
    let channels = channel::get_guild_channels(
        &state,
        &guild_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
    )
    .await?;
    Ok(Json(channels))
}

async fn create_guild_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(guild_id): Path<String>,
    AppJson(payload): AppJson<CreateChannelRequest>,
) -> Result<Json<ChannelResponse>, AppError> {
    let channel = channel::create_guild_channel(
        &state,
        &guild_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        payload,
    )
    .await?;
    Ok(Json(channel))
}

async fn modify_channel_positions(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(guild_id): Path<String>,
    AppJson(payload): AppJson<Vec<ChannelPositionUpdate>>,
) -> Result<StatusCode, AppError> {
    channel::modify_guild_channel_positions(&state, &guild_id, &auth.user_id, payload).await?;
    Ok(StatusCode::NO_CONTENT)
}
