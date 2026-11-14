pub mod channels;
pub mod members;

use axum::{Json, Router, extract::State, routing::post};
use serde::Deserialize;

use crate::AppState;
use crate::error::AppError;
use crate::extractors::AuthUser;
use crate::models::guild::GatewayGuild;
use crate::services::guild_service;

#[derive(Deserialize)]
struct CreateGuildRequest {
    name: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", post(create_guild))
        .nest("/{guild_id}/members", members::router())
        .nest("/{guild_id}/channels", channels::router())
}

async fn create_guild(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<CreateGuildRequest>,
) -> Result<Json<GatewayGuild>, AppError> {
    let guild = guild_service::create_guild(&state, &payload.name, &auth.user_id).await?;
    Ok(Json(guild))
}