pub mod messages;

use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser};
use crate::models::channel::ChannelResponse;
use crate::models::invite::CreatedInvite;
use crate::models::rest::channels::{SetPermissionOverwriteRequest, UpdateChannelRequest};
use crate::models::rest::invites::CreateInviteRequest;
use crate::services::{channel, invite_service};

pub fn router() -> Router<AppState> {
    Router::new()
        .route(
            "/{id}",
            get(get_channel)
                .patch(update_channel)
                .delete(delete_channel),
        )
        .route(
            "/{id}/permissions/{overwrite_id}",
            put(set_permission_overwrite).delete(delete_permission_overwrite),
        )
        .nest("/{id}/messages", messages::router())
        .route("/{id}/invites", post(create_channel_invite))
}

async fn get_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<ChannelResponse>, AppError> {
    let channel = channel::get_channel(&state, &channel_id, &auth.user_id).await?;
    Ok(Json(channel))
}

async fn update_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
    AppJson(payload): AppJson<UpdateChannelRequest>,
) -> Result<Json<ChannelResponse>, AppError> {
    let channel = channel::update_channel(&state, &channel_id, &auth.user_id, payload).await?;
    Ok(Json(channel))
}

async fn delete_channel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<StatusCode, AppError> {
    channel::delete_channel(&state, &channel_id, &auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_permission_overwrite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((channel_id, overwrite_id)): Path<(String, String)>,
    AppJson(payload): AppJson<SetPermissionOverwriteRequest>,
) -> Result<StatusCode, AppError> {
    channel::set_permission_overwrite(&state, &channel_id, &overwrite_id, &auth.user_id, payload)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_permission_overwrite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((channel_id, overwrite_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    channel::delete_permission_overwrite(&state, &channel_id, &overwrite_id, &auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn create_channel_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
    body: Bytes,
) -> Result<Json<CreatedInvite>, AppError> {
    let payload: CreateInviteRequest = if body.is_empty() {
        CreateInviteRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| AppError::invalid_form_body())?
    };

    let invite =
        invite_service::create_channel_invite(&state, &channel_id, &auth.user_id, payload).await?;
    Ok(Json(invite))
}
