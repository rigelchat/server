use std::collections::{HashMap, HashSet};

use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::{
    MAX_ASSET_HASH_LENGTH, MAX_COLOR_VALUE, MAX_GUILD_ROLES, MAX_ROLE_NAME_LENGTH,
    MAX_ROLE_UNICODE_EMOJI_LENGTH,
};
use crate::error::{AppError, ValidationCode};
use crate::models::gateway::events::{GuildRoleBulkEvent, GuildRoleDeleteEvent, GuildRoleEvent};
use crate::models::permission::OverwriteType;
use crate::models::rest::roles::{
    CreateRoleRequest, RoleHoistPositionUpdate, RolePositionUpdate, UpdateRoleRequest,
};
use crate::models::role::{Role, RoleRow};
use crate::services::{member_service, permission_service};
use crate::utils::snowflake::DISCORD_SNOWFLAKE;

pub async fn fetch_guild_roles(state: &AppState, guild_id: &str) -> Result<Vec<Role>, AppError> {
    let rows = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE guild_id = ? ORDER BY position DESC, CAST(id AS UNSIGNED) DESC",
        RoleRow::SELECT_COLS
    ))
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

pub async fn highest_member_role_position<'e, E>(
    executor: E,
    guild_id: &str,
    user_id: &str,
) -> Result<u32, AppError>
where
    E: sqlx::Executor<'e, Database = MySql>,
{
    let pos: Option<u32> = sqlx::query_scalar(
        r#"
        SELECT MAX(r.position)
        FROM roles r
        INNER JOIN guild_member_roles gmr
            ON gmr.role_id = r.id AND gmr.guild_id = r.guild_id
        WHERE r.guild_id = ? AND gmr.user_id = ?
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(executor)
    .await?;

    Ok(pos.unwrap_or(0))
}

pub async fn create_role(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: CreateRoleRequest,
) -> Result<Role, AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    let name = match payload.name {
        Some(raw) => validate_role_name(&raw)?,
        None => "new role".to_string(),
    };

    let color = payload.color.unwrap_or(0);
    if color > MAX_COLOR_VALUE {
        return Err(AppError::validation(
            "color",
            ValidationCode::ColorValueTooHigh,
            "Color value must not exceed 0xffffff.",
        ));
    }

    let hoist = payload.hoist.unwrap_or(false);
    let mentionable = payload.mentionable.unwrap_or(false);

    let icon_val = payload
        .icon
        .as_ref()
        .and_then(|o| o.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let emoji_val = payload
        .unicode_emoji
        .as_ref()
        .and_then(|o| o.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if payload.icon_source_role_id.is_some()
        && (payload.icon.is_some() || payload.unicode_emoji.is_some())
    {
        return Err(AppError::validation(
            "icon_source_role_id",
            ValidationCode::RoleIconSourceAndIconExclusive,
            "Cannot specify icon_source_role_id together with icon or unicode_emoji.",
        ));
    }
    if icon_val.is_some() && emoji_val.is_some() {
        return Err(AppError::validation(
            "icon",
            ValidationCode::RoleIconAndEmojiExclusive,
            "Role cannot have both icon and unicode_emoji.",
        ));
    }

    let mut icon = validate_role_icon_opt(icon_val)?;
    let mut unicode_emoji = validate_role_emoji_opt(emoji_val)?;

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ? FOR UPDATE")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let role_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM roles WHERE guild_id = ?")
        .bind(guild_id)
        .fetch_one(&mut *tx)
        .await?;
    if role_count >= MAX_GUILD_ROLES {
        return Err(AppError::max_guild_roles());
    }

    if let Some(ref src_id) = payload.icon_source_role_id {
        let src: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT icon, unicode_emoji FROM roles WHERE id = ? AND guild_id = ?",
        )
        .bind(src_id)
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((src_icon, src_emoji)) = src else {
            return Err(AppError::unknown_role());
        };
        icon = src_icon;
        unicode_emoji = src_emoji;
    }

    let permissions = match payload.permissions {
        Some(raw) => {
            let truncated = PermissionFlags::from_bits_truncate(raw).bits();
            if user_id != owner_id && (truncated & !actor_perms.bits()) != 0 {
                return Err(AppError::missing_permissions());
            }
            truncated
        }
        None => {
            let everyone_perms: Option<u64> =
                sqlx::query_scalar("SELECT permissions FROM roles WHERE id = ? AND guild_id = ?")
                    .bind(guild_id)
                    .bind(guild_id)
                    .fetch_optional(&mut *tx)
                    .await?;
            everyone_perms.unwrap_or(0)
        }
    };

    let existing_rows = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE guild_id = ? AND id != ? ORDER BY position ASC, CAST(id AS UNSIGNED) ASC FOR UPDATE",
        RoleRow::SELECT_COLS
    ))
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut shifted_roles = Vec::new();
    for (idx, mut r) in existing_rows.into_iter().enumerate() {
        let new_pos = (idx + 2) as u32;
        if r.position != new_pos {
            sqlx::query("UPDATE roles SET position = ? WHERE id = ? AND guild_id = ?")
                .bind(new_pos)
                .bind(&r.id)
                .bind(guild_id)
                .execute(&mut *tx)
                .await?;
            r.position = new_pos;
            shifted_roles.push(Role::from(r));
        }
    }
    shifted_roles.reverse();

    let role_id = DISCORD_SNOWFLAKE.generate(None).to_string();

    sqlx::query(
        r#"
        INSERT INTO roles (id, guild_id, position, name, color, icon, unicode_emoji, hoist, mentionable, permissions)
        VALUES (?, ?, 1, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&role_id)
    .bind(guild_id)
    .bind(&name)
    .bind(color)
    .bind(&icon)
    .bind(&unicode_emoji)
    .bind(hoist)
    .bind(mentionable)
    .bind(permissions)
    .execute(&mut *tx)
    .await?;

    let created_row = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE id = ? AND guild_id = ?",
        RoleRow::SELECT_COLS
    ))
    .bind(&role_id)
    .bind(guild_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    if !shifted_roles.is_empty() {
        state
            .dispatch_guild(
                events::GUILD_ROLE_UPDATE_BULK,
                &GuildRoleBulkEvent {
                    guild_id,
                    roles: &shifted_roles,
                },
                guild_id,
            )
            .await;
    }

    let role = Role::from(created_row);
    state
        .dispatch_guild(
            events::GUILD_ROLE_CREATE,
            &GuildRoleEvent {
                guild_id,
                role: &role,
            },
            guild_id,
        )
        .await;

    Ok(role)
}

pub async fn update_role(
    state: &AppState,
    guild_id: &str,
    role_id: &str,
    user_id: &str,
    payload: UpdateRoleRequest,
) -> Result<Role, AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let current = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE id = ? AND guild_id = ? FOR UPDATE",
        RoleRow::SELECT_COLS
    ))
    .bind(role_id)
    .bind(guild_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(AppError::unknown_role)?;

    if user_id != owner_id {
        let highest_pos = highest_member_role_position(&mut *tx, guild_id, user_id).await?;
        if highest_pos <= current.position {
            return Err(AppError::missing_permissions());
        }
    }

    let is_everyone = role_id == guild_id;

    let icon_non_null = payload
        .icon
        .as_ref()
        .and_then(|o| o.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let emoji_non_null = payload
        .unicode_emoji
        .as_ref()
        .and_then(|o| o.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());

    if is_everyone {
        if icon_non_null.is_some() {
            return Err(AppError::validation(
                "icon",
                ValidationCode::CannotSetIconOnEveryoneRole,
                "Cannot set icon on the @everyone role.",
            ));
        }
        if emoji_non_null.is_some() {
            return Err(AppError::validation(
                "unicode_emoji",
                ValidationCode::CannotSetIconOnEveryoneRole,
                "Cannot set unicode_emoji on the @everyone role.",
            ));
        }
    }

    if icon_non_null.is_some() && emoji_non_null.is_some() {
        return Err(AppError::validation(
            "icon",
            ValidationCode::RoleIconAndEmojiExclusive,
            "Role cannot have both icon and unicode_emoji.",
        ));
    }

    let mut query = QueryBuilder::<MySql>::new("UPDATE roles SET ");
    let mut sep = query.separated(", ");
    let mut has_updates = false;
    let mut needs_member_list_refresh = false;

    if !is_everyone && let Some(raw_name) = &payload.name {
        let name = validate_role_name(raw_name)?;
        sep.push("name = ");
        sep.push_bind_unseparated(name);
        has_updates = true;
    }

    if let Some(color) = payload.color {
        if color > MAX_COLOR_VALUE {
            return Err(AppError::validation(
                "color",
                ValidationCode::ColorValueTooHigh,
                "Color value must not exceed 0xffffff.",
            ));
        }
        sep.push("color = ");
        sep.push_bind_unseparated(color);
        has_updates = true;
    }

    if let Some(raw_perms) = payload.permissions {
        let new_perms = PermissionFlags::from_bits_truncate(raw_perms).bits();
        let added = new_perms & !current.permissions;
        if user_id != owner_id && (added & !actor_perms.bits()) != 0 {
            return Err(AppError::missing_permissions());
        }
        if new_perms != current.permissions {
            needs_member_list_refresh = true;
        }
        sep.push("permissions = ");
        sep.push_bind_unseparated(new_perms);
        has_updates = true;
    }

    if !is_everyone && let Some(hoist) = payload.hoist {
        if hoist != current.hoist {
            needs_member_list_refresh = true;
        }
        sep.push("hoist = ");
        sep.push_bind_unseparated(hoist);
        has_updates = true;
    }

    if !is_everyone && let Some(hoist_pos) = payload.hoist_position {
        if hoist_pos != current.hoist_position {
            needs_member_list_refresh = true;
        }
        sep.push("hoist_position = ");
        sep.push_bind_unseparated(hoist_pos);
        has_updates = true;
    }

    if !is_everyone && let Some(mentionable) = payload.mentionable {
        sep.push("mentionable = ");
        sep.push_bind_unseparated(mentionable);
        has_updates = true;
    }

    if let Some(val) = icon_non_null {
        let validated = validate_role_icon_opt(Some(val))?;
        sep.push("icon = ");
        sep.push_bind_unseparated(validated);
        sep.push("unicode_emoji = ");
        sep.push_bind_unseparated(Option::<String>::None);
        has_updates = true;
    } else if let Some(val) = emoji_non_null {
        let validated = validate_role_emoji_opt(Some(val))?;
        sep.push("unicode_emoji = ");
        sep.push_bind_unseparated(validated);
        sep.push("icon = ");
        sep.push_bind_unseparated(Option::<String>::None);
        has_updates = true;
    } else {
        if payload.icon.is_some() {
            sep.push("icon = ");
            sep.push_bind_unseparated(Option::<String>::None);
            has_updates = true;
        }
        if payload.unicode_emoji.is_some() {
            sep.push("unicode_emoji = ");
            sep.push_bind_unseparated(Option::<String>::None);
            has_updates = true;
        }
    }

    if !has_updates {
        tx.commit().await?;
        return Ok(Role::from(current));
    }

    query.push(" WHERE id = ");
    query.push_bind(role_id);
    query.push(" AND guild_id = ");
    query.push_bind(guild_id);
    query.build().execute(&mut *tx).await?;

    let updated_row = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE id = ? AND guild_id = ?",
        RoleRow::SELECT_COLS
    ))
    .bind(role_id)
    .bind(guild_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let updated = Role::from(updated_row);
    state
        .dispatch_guild(
            events::GUILD_ROLE_UPDATE,
            &GuildRoleEvent {
                guild_id,
                role: &updated,
            },
            guild_id,
        )
        .await;

    if needs_member_list_refresh {
        crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;
    }

    Ok(updated)
}

pub async fn delete_role(
    state: &AppState,
    guild_id: &str,
    role_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    if role_id == guild_id {
        return Err(AppError::missing_permissions_with(
            "Cannot delete the @everyone role.",
        ));
    }

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let role_pos: Option<u32> =
        sqlx::query_scalar("SELECT position FROM roles WHERE id = ? AND guild_id = ? FOR UPDATE")
            .bind(role_id)
            .bind(guild_id)
            .fetch_optional(&mut *tx)
            .await?;

    let Some(target_pos) = role_pos else {
        return Err(AppError::unknown_role());
    };

    if user_id != owner_id {
        let highest_pos = highest_member_role_position(&mut *tx, guild_id, user_id).await?;
        if highest_pos <= target_pos {
            return Err(AppError::missing_permissions());
        }
    }

    sqlx::query(
        r#"
        DELETE o FROM channel_permission_overwrites o
        INNER JOIN channels c ON c.id = o.channel_id
        WHERE c.guild_id = ? AND o.type = ? AND o.target_id = ?
        "#,
    )
    .bind(guild_id)
    .bind(OverwriteType::Role as u32)
    .bind(role_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM roles WHERE id = ? AND guild_id = ?")
        .bind(role_id)
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;

    let remaining_rows = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE guild_id = ? AND id != ? ORDER BY position ASC, CAST(id AS UNSIGNED) ASC FOR UPDATE",
        RoleRow::SELECT_COLS
    ))
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut shifted_roles = Vec::new();
    for (idx, mut r) in remaining_rows.into_iter().enumerate() {
        let new_pos = (idx + 1) as u32;
        if r.position != new_pos {
            sqlx::query("UPDATE roles SET position = ? WHERE id = ? AND guild_id = ?")
                .bind(new_pos)
                .bind(&r.id)
                .bind(guild_id)
                .execute(&mut *tx)
                .await?;
            r.position = new_pos;
            shifted_roles.push(Role::from(r));
        }
    }
    shifted_roles.reverse();

    tx.commit().await?;

    state
        .dispatch_guild(
            events::GUILD_ROLE_DELETE,
            &GuildRoleDeleteEvent { guild_id, role_id },
            guild_id,
        )
        .await;

    if !shifted_roles.is_empty() {
        state
            .dispatch_guild(
                events::GUILD_ROLE_UPDATE_BULK,
                &GuildRoleBulkEvent {
                    guild_id,
                    roles: &shifted_roles,
                },
                guild_id,
            )
            .await;
    }

    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;

    Ok(())
}

pub async fn modify_role_positions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: Vec<RolePositionUpdate>,
) -> Result<(), AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    if payload.is_empty() {
        return Ok(());
    }

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let is_owner = user_id == owner_id;
    let highest_pos = if is_owner {
        u32::MAX
    } else {
        highest_member_role_position(&mut *tx, guild_id, user_id).await?
    };

    let rows = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE guild_id = ? AND id != ? ORDER BY position ASC, CAST(id AS UNSIGNED) ASC FOR UPDATE",
        RoleRow::SELECT_COLS
    ))
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;

    let by_id: HashMap<String, &RoleRow> = rows.iter().map(|r| (r.id.clone(), r)).collect();

    for entry in &payload {
        if entry.id == guild_id {
            return Err(AppError::validation(
                "id",
                ValidationCode::CannotReorderEveryoneRole,
                "Cannot reorder the @everyone role.",
            ));
        }
        let Some(role) = by_id.get(&entry.id) else {
            return Err(AppError::validation(
                "id",
                ValidationCode::InvalidRoleId,
                "Role does not exist in this community.",
            ));
        };
        if !is_owner
            && highest_pos <= role.position
            && entry.position.is_some_and(|pos| pos != role.position as i32)
        {
            return Err(AppError::missing_permissions());
        }
    }

    let (updates, final_roles) =
        compute_reordered_positions(&rows, &payload, is_owner, highest_pos);
    if updates.is_empty() {
        tx.commit().await?;
        return Ok(());
    }

    for (id, new_pos) in updates {
        sqlx::query("UPDATE roles SET position = ? WHERE id = ? AND guild_id = ?")
            .bind(new_pos)
            .bind(&id)
            .bind(guild_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await?;

    state
        .dispatch_guild(
            events::GUILD_ROLE_UPDATE_BULK,
            &GuildRoleBulkEvent {
                guild_id,
                roles: &final_roles,
            },
            guild_id,
        )
        .await;

    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;

    Ok(())
}

pub async fn modify_role_hoist_positions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: Vec<RoleHoistPositionUpdate>,
) -> Result<(), AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    if payload.is_empty() {
        return Ok(());
    }

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let is_owner = user_id == owner_id;
    let highest_pos = if is_owner {
        u32::MAX
    } else {
        highest_member_role_position(&mut *tx, guild_id, user_id).await?
    };

    let rows = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE guild_id = ? AND id != ? FOR UPDATE",
        RoleRow::SELECT_COLS
    ))
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;

    let by_id: HashMap<String, RoleRow> = rows.into_iter().map(|r| (r.id.clone(), r)).collect();

    let mut final_hoist: HashMap<String, i32> = HashMap::new();
    for entry in payload {
        if entry.id == guild_id {
            return Err(AppError::validation(
                "id",
                ValidationCode::CannotReorderEveryoneRole,
                "Cannot change hoist position of the @everyone role.",
            ));
        }
        let Some(role) = by_id.get(&entry.id) else {
            return Err(AppError::validation(
                "id",
                ValidationCode::InvalidRoleId,
                "Role does not exist in this community.",
            ));
        };
        if !is_owner && highest_pos <= role.position {
            return Err(AppError::missing_permissions());
        }
        final_hoist.insert(entry.id, entry.hoist_position);
    }

    let mut changed_roles = Vec::new();
    for (id, hoist_pos) in final_hoist {
        if let Some(orig) = by_id.get(&id)
            && orig.hoist_position != Some(hoist_pos)
        {
            sqlx::query("UPDATE roles SET hoist_position = ? WHERE id = ? AND guild_id = ?")
                .bind(hoist_pos)
                .bind(&id)
                .bind(guild_id)
                .execute(&mut *tx)
                .await?;
            let mut updated = Role::from(orig.clone());
            updated.hoist_position = Some(hoist_pos);
            changed_roles.push(updated);
        }
    }

    tx.commit().await?;

    if !changed_roles.is_empty() {
        state
            .dispatch_guild(
                events::GUILD_ROLE_UPDATE_BULK,
                &GuildRoleBulkEvent {
                    guild_id,
                    roles: &changed_roles,
                },
                guild_id,
            )
            .await;
        crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;
    }

    Ok(())
}

pub async fn reset_role_hoist_positions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    let mut tx = state.db.begin().await?;

    let rows = sqlx::query_as::<_, RoleRow>(&format!(
        "SELECT {} FROM roles WHERE guild_id = ? AND hoist_position IS NOT NULL FOR UPDATE",
        RoleRow::SELECT_COLS
    ))
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;

    if rows.is_empty() {
        tx.commit().await?;
        return Ok(());
    }

    sqlx::query(
        "UPDATE roles SET hoist_position = NULL WHERE guild_id = ? AND hoist_position IS NOT NULL",
    )
    .bind(guild_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let cleared: Vec<Role> = rows
        .into_iter()
        .map(|mut r| {
            r.hoist_position = None;
            Role::from(r)
        })
        .collect();

    state
        .dispatch_guild(
            events::GUILD_ROLE_UPDATE_BULK,
            &GuildRoleBulkEvent {
                guild_id,
                roles: &cleared,
            },
            guild_id,
        )
        .await;
    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;

    Ok(())
}

fn compute_reordered_positions(
    sorted_roles: &[RoleRow],
    payload: &[RolePositionUpdate],
    is_owner: bool,
    highest_pos: u32,
) -> (Vec<(String, u32)>, Vec<Role>) {
    let mut requested: HashMap<&str, (Option<i32>, usize)> = HashMap::new();
    for (idx, item) in payload.iter().enumerate() {
        requested.insert(item.id.as_str(), (item.position, idx));
    }

    let mut manageable_slots: Vec<usize> = Vec::new();
    let mut manageable_items: Vec<(i32, i8, usize, &RoleRow)> = Vec::new();
    let mut seen_ids = HashSet::new();

    for (slot_idx, role) in sorted_roles.iter().enumerate() {
        if !seen_ids.insert(role.id.as_str()) {
            continue;
        }
        if is_owner || highest_pos > role.position {
            manageable_slots.push(slot_idx);
            let current_pos = role.position as i32;
            let (target_pos, bias, req_order) = match requested.get(role.id.as_str()) {
                Some(&(Some(pos), order)) => {
                    let b = if pos > current_pos {
                        1
                    } else if pos < current_pos {
                        -1
                    } else {
                        0
                    };
                    (pos, b, order)
                }
                Some(&(None, order)) => (current_pos, 0, order),
                None => (current_pos, 0, usize::MAX),
            };
            manageable_items.push((target_pos, bias, req_order, role));
        }
    }

    manageable_items.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.position.cmp(&b.3.position))
    });

    let mut placed: Vec<&RoleRow> = sorted_roles.iter().collect();
    for (slot_idx, (_, _, _, role)) in manageable_slots.into_iter().zip(manageable_items) {
        placed[slot_idx] = role;
    }

    let mut updates = Vec::new();
    let mut final_roles = Vec::with_capacity(placed.len());
    for (idx, role) in placed.into_iter().enumerate() {
        let new_pos = (idx + 1) as u32;
        if role.position != new_pos {
            updates.push((role.id.clone(), new_pos));
        }
        let mut updated = Role::from(role.clone());
        updated.position = new_pos;
        final_roles.push(updated);
    }
    final_roles.reverse();
    (updates, final_roles)
}

fn validate_role_name(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_ROLE_NAME_LENGTH {
        return Err(AppError::validation(
            "name",
            ValidationCode::StringLengthInvalid,
            "Role name must be between 1 and 100 characters.",
        ));
    }
    Ok(trimmed.to_string())
}

fn validate_role_icon_opt(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.len() > MAX_ASSET_HASH_LENGTH
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::validation(
            "icon",
            ValidationCode::InvalidFormat,
            "Invalid icon identifier.",
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_role_emoji_opt(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.len() > MAX_ROLE_UNICODE_EMOJI_LENGTH || trimmed.chars().any(char::is_control) {
        return Err(AppError::validation(
            "unicode_emoji",
            ValidationCode::StringLengthInvalid,
            "Invalid unicode_emoji.",
        ));
    }
    Ok(Some(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn role_row(id: &str, pos: u32) -> RoleRow {
        RoleRow {
            id: id.to_string(),
            position: pos,
            name: id.to_string(),
            color: 0,
            hoist_position: None,
            icon: None,
            unicode_emoji: None,
            hoist: false,
            mentionable: false,
            permissions: 0,
        }
    }

    #[test]
    fn reorders_full_and_partial_role_moves_respecting_hierarchy() {
        let rows = vec![role_row("r1", 1), role_row("r2", 2), role_row("r3", 3)];

        let full = vec![
            RolePositionUpdate { id: "r3".into(), position: Some(1) },
            RolePositionUpdate { id: "r2".into(), position: Some(2) },
            RolePositionUpdate { id: "r1".into(), position: Some(3) },
        ];
        let (updates, final_roles) = compute_reordered_positions(&rows, &full, true, u32::MAX);
        assert_eq!(updates, vec![("r3".into(), 1), ("r1".into(), 3)]);
        assert_eq!(
            final_roles.iter().map(|r| (r.id.as_str(), r.position)).collect::<Vec<_>>(),
            vec![("r1", 3), ("r2", 2), ("r3", 1)]
        );

        let move_up = vec![RolePositionUpdate { id: "r1".into(), position: Some(3) }];
        let (updates, _) = compute_reordered_positions(&rows, &move_up, true, u32::MAX);
        assert_eq!(
            updates,
            vec![("r2".into(), 1), ("r3".into(), 2), ("r1".into(), 3)]
        );

        let bounded = vec![RolePositionUpdate { id: "r1".into(), position: Some(2) }];
        let (updates, _) = compute_reordered_positions(&rows, &bounded, false, 3);
        assert_eq!(updates, vec![("r2".into(), 1), ("r1".into(), 2)]);
    }
}
