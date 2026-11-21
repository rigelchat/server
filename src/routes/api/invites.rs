use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::invite::InviteResponse;
use crate::services::invite_service;

pub fn router() -> Router<AppState> {
    Router::new().route(
        "/{code}",
        get(get_invite).post(accept_invite).delete(delete_invite),
    )
}

async fn get_invite(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<InviteResponse>, AppError> {
    let invite = invite_service::get_invite(&state, &code).await?;
    Ok(Json(invite))
}

async fn accept_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(code): Path<String>,
) -> Result<Json<InviteResponse>, AppError> {
    let invite = invite_service::accept_invite(&state, &code, &auth.user_id).await?;
    Ok(Json(invite))
}

async fn delete_invite(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(code): Path<String>,
) -> Result<StatusCode, AppError> {
    invite_service::delete_invite(&state, &code, &auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
