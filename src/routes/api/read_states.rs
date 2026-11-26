use axum::{Json, Router, extract::State, http::StatusCode, routing::post};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser, FluxerFeatures};
use crate::models::rest::read_states::{
    AckReadStatesRequest, AckReadStatesResponse, BulkAckReadStatesRequest,
};
use crate::services::read_state_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/ack", post(ack_read_states))
        .route("/ack-bulk", post(ack_read_states_bulk))
}

async fn ack_read_states(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    AppJson(payload): AppJson<AckReadStatesRequest>,
) -> Result<Json<AckReadStatesResponse>, AppError> {
    let res = read_state_service::ack_read_states(
        &state,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        auth.is_bot,
        payload,
    )
    .await?;
    Ok(Json(res))
}

async fn ack_read_states_bulk(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    AppJson(payload): AppJson<BulkAckReadStatesRequest>,
) -> Result<StatusCode, AppError> {
    read_state_service::ack_read_states_bulk(
        &state,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        auth.is_bot,
        payload,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
