use crate::AppState;
use crate::error::AppError;
use crate::models::role::{Role, RoleRow};
use crate::services::member_service;

pub async fn fetch_guild_roles(state: &AppState, guild_id: &str) -> Result<Vec<Role>, AppError> {
    let rows = sqlx::query_as::<_, RoleRow>(
        "SELECT id, position, name, color, unicode_emoji, hoist, mentionable, permissions FROM roles WHERE guild_id = ? ORDER BY position DESC",
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    Ok(rows.into_iter().map(Role::from).collect())
}

pub async fn get_guild_roles(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<Vec<Role>, AppError> {
    member_service::require_member(state, guild_id, user_id).await?;
    fetch_guild_roles(state, guild_id).await
}
