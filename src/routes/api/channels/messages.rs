use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser, FluxerFeatures};
use crate::models::message::Message;
use crate::models::rest::messages::{CreateMessageRequest, EditMessageRequest, GetMessagesQuery};
use crate::models::rest::read_states::{AckMessageRequest, AckMessageResponse};
use crate::services::{message_service, read_state_service};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, patch, post},
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_messages).post(create_message))
        .route("/{message_id}", patch(edit_message).delete(delete_message))
        .route("/{message_id}/ack", post(ack_message))
}

async fn get_messages(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
    Query(query): Query<GetMessagesQuery>,
) -> Result<Json<Vec<Message>>, AppError> {
    let messages = message_service::get_messages(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        query,
    )
    .await?;
    Ok(Json(messages))
}

async fn create_message(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
    AppJson(payload): AppJson<CreateMessageRequest>,
) -> Result<Json<Message>, AppError> {
    let msg = message_service::create_message(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        auth.is_bot,
        payload,
    )
    .await?;
    Ok(Json(msg))
}

async fn edit_message(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path((channel_id, message_id)): Path<(String, String)>,
    AppJson(payload): AppJson<EditMessageRequest>,
) -> Result<Json<Message>, AppError> {
    let updated = message_service::edit_message(
        &state,
        &channel_id,
        &message_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        payload,
    )
    .await?;
    Ok(Json(updated))
}

async fn delete_message(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path((channel_id, message_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    message_service::delete_message(
        &state,
        &channel_id,
        &message_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ack_message(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path((channel_id, message_id)): Path<(String, String)>,
    body: Bytes,
) -> Result<Json<AckMessageResponse>, AppError> {
    let payload: AckMessageRequest = if body.is_empty() {
        AckMessageRequest::default()
    } else {
        serde_json::from_slice(&body).map_err(|_| AppError::invalid_form_body())?
    };

    let res = read_state_service::ack_message(
        &state,
        &channel_id,
        &message_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        auth.is_bot,
        payload,
    )
    .await?;
    Ok(Json(res))
}
