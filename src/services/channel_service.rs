use std::collections::HashMap;

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::error::AppError;
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, GuildChannel, PermissionOverwrite,
};
use crate::services::{member_service, permission_service};

pub async fn get_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> Result<ChannelResponse, AppError> {
    let channel = sqlx::query_as::<_, GuildChannel>(
        "SELECT id, type, position, guild_id, parent_id, name, last_message_id FROM channels WHERE id = ?",
    )
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    member_service::require_member(state, &channel.guild_id, user_id).await?;
    permission_service::require_channel_permission(
        state,
        &channel.guild_id,
        &channel.id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await?;

    let overwrites = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(PermissionOverwrite::from)
    .collect();

    Ok(ChannelResponse::from_parts(channel, overwrites))
}

pub async fn fetch_guild_channels(
    state: &AppState,
    guild_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    let channels = sqlx::query_as::<_, GuildChannel>(
        "SELECT id, type, position, guild_id, parent_id, name, last_message_id FROM channels WHERE guild_id = ? ORDER BY position ASC",
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    if channels.is_empty() {
        return Ok(vec![]);
    }

    let overwrites = sqlx::query_as::<_, ChannelOverwriteRow>(
        r#"
        SELECT o.channel_id, o.type, o.target_id, o.allow, o.deny
        FROM channel_permission_overwrites o
        INNER JOIN channels c ON c.id = o.channel_id
        WHERE c.guild_id = ?
        "#,
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    let mut overwrites_by_channel: HashMap<String, Vec<PermissionOverwrite>> = HashMap::new();
    for ow in overwrites {
        overwrites_by_channel
            .entry(ow.channel_id.clone())
            .or_default()
            .push(PermissionOverwrite::from(ow));
    }

    Ok(channels
        .into_iter()
        .map(|base| {
            let permission_overwrites = overwrites_by_channel.remove(&base.id).unwrap_or_default();
            ChannelResponse::from_parts(base, permission_overwrites)
        })
        .collect())
}

pub async fn get_guild_channels(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    member_service::require_member(state, guild_id, user_id).await?;
    let channels = fetch_guild_channels(state, guild_id).await?;
    permission_service::filter_visible_channels(state, guild_id, user_id, channels).await
}