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
    pub is_bot: bool,
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

        let Some(is_bot) =
            crate::services::auth_service::is_session_active(&state, &session_id, &user_id).await?
        else {
            return Err(AppError::invalid_token(
                "Invalid or expired authorization token",
            ));
        };

        Ok(Self {
            user_id,
            session_id,
            is_bot,
        })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FluxerFeatures {
    pub channel_threads: bool,
    pub view_channel_members_permission: bool,
}

impl FluxerFeatures {
    pub const fn has_channel_threads(self, is_bot: bool) -> bool {
        is_bot || self.channel_threads
    }
}

impl<S> FromRequestParts<S> for FluxerFeatures
where
    S: Send + Sync,
{
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let mut features = Self::default();
        if let Some(raw) = parts
            .headers
            .get("x-fluxer-features")
            .and_then(|v| v.to_str().ok())
        {
            for token in raw.split(',').map(str::trim) {
                match token {
                    "channel_threads" => features.channel_threads = true,
                    "view_channel_members_permission" => {
                        features.view_channel_members_permission = true;
                    }
                    _ => {}
                }
            }
        }
        Ok(features)
    }
}
