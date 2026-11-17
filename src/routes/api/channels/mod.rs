pub mod messages;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::error::{ApiErrorCode, AppError};
use crate::extractors::AuthUser;
use crate::models::channel::ChannelResponse;
use crate::models::invite::{CreateInviteRequest, CreatedInvite};
use crate::services::{guild_service, invite_service};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{id}", get(get_channel))
        .nest("/{id}/messages", messages::router())
        .route("/{id}/invites", post(create_channel_invite))
}

async fn get_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<ChannelResponse>, AppError> {
    let channel = guild_service::get_channel(&state, &channel_id, &auth.user_id).await?;
    Ok(Json(channel))
}

async fn create_channel_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    dialect: ClientDialect,
    Path(channel_id): Path<String>,
    body: Bytes,
) -> Result<Json<CreatedInvite>, AppError> {
    let payload: CreateInviteRequest = if body.is_empty() {
        CreateInviteRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| {
            AppError::coded(
                StatusCode::BAD_REQUEST,
                ApiErrorCode::InvalidFormBody,
                "Invalid form body.",
            )
        })?
    };

    let invite =
        invite_service::create_channel_invite(&state, &channel_id, &auth.user_id, payload).await?;
    Ok(Json(invite.with_dialect(dialect)))
}