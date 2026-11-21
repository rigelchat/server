use axum::{
    Json,
    extract::{FromRef, FromRequest, FromRequestParts, Request, rejection::JsonRejection},
    http::{header::AUTHORIZATION, request::Parts},
};

use crate::{
    error::{AppError, ValidationCode},
    state::AppState,
    utils::token::{self, TokenClaims},
};

pub struct AppJson<T>(pub T);

impl<S, T> FromRequest<S> for AppJson<T>
where
    Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(value)) => Ok(Self(value)),
            Err(rejection) => Err(AppError::validation(
                "root",
                ValidationCode::InvalidFormat,
                rejection.to_string(),
            )),
        }
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
            .ok_or_else(AppError::missing_auth)?;

        let token = token::strip_auth_prefix(raw_header);

        let TokenClaims {
            user_id,
            session_id,
        } = token::verify(token, &state.config.auth.secret).map_err(AppError::invalid_token)?;

        if !crate::services::auth_service::is_session_active(&state, &session_id, &user_id).await?
        {
            return Err(AppError::invalid_token(
                "Invalid or expired authorization token",
            ));
        }

        Ok(Self {
            user_id,
            session_id,
        })
    }
}