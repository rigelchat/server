use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch},
};
use serde::Deserialize;

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::message::{GetMessagesQuery, Message};
use crate::models::serde_helpers::deserialize_opt_string_or_int;
use crate::services::message_service;

#[derive(Deserialize)]
struct CreateMessageRequest {
    content: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
    nonce: Option<String>,
}

#[derive(Deserialize)]
struct EditMessageRequest {
    content: Option<String>,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_messages).post(create_message))
        .route("/{message_id}", patch(edit_message).delete(delete_message))
}

async fn get_messages(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<Json<Vec<Message>>, AppError> {
    let messages =
        message_service::get_messages(&state, &channel_id, &auth.user_id, query).await?;
    Ok(Json(messages))
}

async fn create_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
    Json(payload): Json<CreateMessageRequest>,
) -> Result<Json<Message>, AppError> {
    let msg = message_service::create_message(
        &state,
        &channel_id,
        &auth.user_id,
        payload.content.unwrap_or_default(),
        payload.nonce,
    )
    .await?;
    Ok(Json(msg))
}

async fn edit_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((channel_id, message_id)): Path<(String, String)>,
    Json(payload): Json<EditMessageRequest>,
) -> Result<Json<Message>, AppError> {
    let updated = message_service::edit_message(
        &state,
        &channel_id,
        &message_id,
        &auth.user_id,
        payload.content.unwrap_or_default(),
    )
    .await?;
    Ok(Json(updated))
}

async fn delete_message(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    message_service::delete_message(&state, &channel_id, &message_id, &auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}