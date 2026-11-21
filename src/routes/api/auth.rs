use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser};
use crate::models::rest::auth::{
    LoginRequest, LoginResponse, RegisterRequest, UsernameAvailabilityQuery,
    UsernameAvailabilityResponse, UsernameSuggestionsRequest, UsernameSuggestionsResponse,
};
use crate::services::auth_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/register", post(register))
        .route("/logout", post(logout))
        .route("/username-availability", get(username_availability))
        .route("/username-suggestions", post(username_suggestions))
}

async fn login(
    State(state): State<AppState>,
    AppJson(payload): AppJson<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let res = auth_service::login(&state, payload).await?;
    Ok(Json(res))
}

async fn register(
    State(state): State<AppState>,
    AppJson(payload): AppJson<RegisterRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    let res = auth_service::register(&state, payload).await?;
    Ok(Json(res))
}

async fn logout(State(state): State<AppState>, auth: AuthUser) -> Result<StatusCode, AppError> {
    auth_service::logout(&state, &auth.user_id, &auth.session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn username_availability(
    State(state): State<AppState>,
    Query(query): Query<UsernameAvailabilityQuery>,
) -> Result<Json<UsernameAvailabilityResponse>, AppError> {
    let available = auth_service::is_username_available(&state, &query.username).await?;
    Ok(Json(UsernameAvailabilityResponse { available }))
}

async fn username_suggestions(
    AppJson(payload): AppJson<UsernameSuggestionsRequest>,
) -> Json<UsernameSuggestionsResponse> {
    let suggestion = auth_service::suggest_username_from_display(&payload.global_name);
    Json(UsernameSuggestionsResponse {
        suggestions: vec![suggestion],
    })
}