pub mod messages;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use serde_json::Value;

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::services::invite_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/{id}/messages", messages::router())
        .route("/{id}/invites", post(create_channel_invite))
}

async fn create_channel_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
) -> Result<Json<Value>, AppError> {
    let invite =
        invite_service::create_channel_invite(&state, &channel_id, &auth.user_id).await?;
    Ok(Json(invite))
}