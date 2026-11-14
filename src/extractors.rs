use axum::{
    extract::{FromRef, FromRequestParts},
    http::{header::AUTHORIZATION, request::Parts},
};

use crate::{
    error::AppError,
    state::AppState,
    utils::token::{self, TokenClaims},
};

pub struct AuthUser {
    pub user_id: String,
    pub session_id: String,
}

impl<S> FromRequestParts<S> for AuthUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let state = AppState::from_ref(state);

        let raw_header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Unauthorized("Missing authorization header".to_string()))?;

        let token = raw_header
            .strip_prefix("Bearer ")
            .unwrap_or(raw_header)
            .trim();

        let TokenClaims { user_id, session_id } =
            token::verify(token, &state.config.auth_secret).map_err(AppError::Unauthorized)?;

        Ok(Self {
            user_id,
            session_id
        })
    }
}