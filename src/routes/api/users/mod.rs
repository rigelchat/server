pub mod me;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::rest::users::{GetProfileQuery, UserProfileResponse};
use crate::models::user::PartialUser;
use crate::services::user_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/@me", me::router())
        .route("/{id}", get(get_user))
        .route("/{id}/profile", get(get_user_profile))
}

async fn get_user(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<PartialUser>, AppError> {
    let user = user_service::get_partial_user(&state, &user_id).await?;
    Ok(Json(user))
}

async fn get_user_profile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(user_id): Path<String>,
    Query(query): Query<GetProfileQuery>,
) -> Result<Json<UserProfileResponse>, AppError> {
    let profile = user_service::get_profile(&state, &auth.user_id, &user_id, query).await?;
    Ok(Json(profile))
}