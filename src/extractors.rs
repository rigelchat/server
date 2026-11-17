use axum::{
    extract::{FromRef, FromRequestParts},
    http::{StatusCode, header::AUTHORIZATION, request::Parts},
};

use crate::{
    constants::dialect::ClientDialect,
    error::{ApiErrorCode, AppError},
    state::AppState,
    utils::token::{self, TokenClaims},
};

impl<S> FromRequestParts<S> for ClientDialect
where
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(dialect) = parts.extensions.get::<ClientDialect>().copied() {
            return Ok(dialect);
        }

        Ok(ClientDialect::from_http(parts.uri.path(), &parts.headers))
    }
}

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
            .ok_or_else(|| {
                AppError::coded(
                    StatusCode::UNAUTHORIZED,
                    ApiErrorCode::MissingAuthorization,
                    "Missing or invalid authorization header",
                )
            })?;

        let token = raw_header
            .strip_prefix("Bearer ")
            .or_else(|| raw_header.strip_prefix("Bot "))
            .unwrap_or(raw_header)
            .trim();

        let TokenClaims {
            user_id,
            session_id,
        } = token::verify(token, &state.config.auth_secret).map_err(|err| {
            AppError::coded(
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::InvalidAuthToken,
                err,
            )
        })?;

        if !crate::services::auth_service::is_session_active(&state, &session_id, &user_id).await? {
            return Err(AppError::coded(
                StatusCode::UNAUTHORIZED,
                ApiErrorCode::InvalidAuthToken,
                "Invalid or expired authorization token",
            ));
        }

        Ok(Self {
            user_id,
            session_id,
        })
    }
}