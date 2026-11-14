pub mod me;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::rest::users::UserProfileResponse;
use crate::services::user_service;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/@me", me::router())
        .route("/{id}/profile", get(get_user_profile))
}

async fn get_user_profile(
    State(state): State<AppState>,
    _auth: AuthUser,
    Path(user_id): Path<String>,
) -> Result<Json<UserProfileResponse>, AppError> {
    let profile = user_service::get_profile(&state, &user_id).await?;
    Ok(Json(profile))
}