use axum::{Json, Router, extract::State, routing::post};

use crate::AppState;
use crate::error::AppError;
use crate::models::rest::auth::{LoginRequest, LoginResponse, RegisterRequest};
use crate::services::auth_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/register", post(register))
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let res = auth_service::login(&state, payload).await?;
    Ok(Json(res))
}

async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let res = auth_service::register(&state, payload).await?;
    Ok(Json(res))
}