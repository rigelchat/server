use std::collections::HashMap;

use sqlx::{Executor, MySql};

use crate::error::AppError;
use crate::models::channel::{ChannelOverwriteRow, PermissionOverwrite};
use crate::models::guild_member::GuildMemberRole;

pub async fn fetch_member_roles<'e, E>(
    executor: E,
    guild_id: &str,
    user_id: &str,
) -> Result<Vec<String>, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    let roles = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(guild_id)
    .fetch_all(executor)
    .await?;

    Ok(roles)
}

pub async fn fetch_guild_member_roles_map<'e, E>(
    executor: E,
    guild_id: &str,
) -> Result<HashMap<String, Vec<String>>, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    let rows = sqlx::query_as::<_, GuildMemberRole>(
        "SELECT user_id, role_id FROM guild_member_roles WHERE guild_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(executor)
    .await?;

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    for r in rows {
        map.entry(r.user_id).or_default().push(r.role_id);
    }
    Ok(map)
}

pub async fn fetch_channel_overwrites<'e, E>(
    executor: E,
    channel_id: &str,
) -> Result<Vec<PermissionOverwrite>, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    let rows = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(PermissionOverwrite::from).collect())
}
