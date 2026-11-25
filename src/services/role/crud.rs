use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::OverwriteType;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::{
    MAX_COLOR_VALUE, MAX_GUILD_ROLES, MAX_ROLE_NAME_LENGTH, MAX_ROLE_UNICODE_EMOJI_LENGTH,
};
use crate::error::{AppError, ValidationCode};
use crate::models::gateway::events::{GuildRoleBulkEvent, GuildRoleDeleteEvent, GuildRoleEvent};
use crate::models::rest::roles::{CreateRoleRequest, UpdateRoleRequest};
use crate::models::role::{Role, RoleRow};
use crate::services::{member_service, permission_service};
use crate::utils::snowflake::DISCORD_SNOWFLAKE;
use crate::utils::validation::{reject_control_chars, validate_asset_opt};

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
    can_see_thread_perms: bool,
) -> Result<Vec<Role>, AppError> {
    member_service::require_member(state, guild_id, user_id).await?;
    let mut roles = fetch_guild_roles(state, guild_id).await?;
    if !can_see_thread_perms {
        let mask = !PermissionFlags::THREAD_GATED_BITS.bits();
        for r in &mut roles {
            r.permissions &= mask;
        }
    }
    Ok(roles)
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
    can_write_thread_perms: bool,
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

    let mut icon = validate_asset_opt("icon", icon_val)?;
    let mut unicode_emoji = validate_role_emoji_opt(emoji_val)?;

    let mut tx = state.db.begin().await?;

    let owner_id: String =
        sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ? FOR UPDATE")
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
        let src: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT icon, unicode_emoji FROM roles WHERE id = ? AND guild_id = ?")
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
            let mut truncated = PermissionFlags::from_bits_truncate(raw).bits();
            if !can_write_thread_perms {
                truncated &= !PermissionFlags::THREAD_GATED_BITS.bits();
            }
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

    let mut role = Role::from(created_row);
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

    if !can_write_thread_perms {
        role.permissions &= !PermissionFlags::THREAD_GATED_BITS.bits();
    }
    Ok(role)
}

pub async fn update_role(
    state: &AppState,
    guild_id: &str,
    role_id: &str,
    user_id: &str,
    can_write_thread_perms: bool,
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
        let truncated = PermissionFlags::from_bits_truncate(raw_perms).bits();
        let new_perms = if can_write_thread_perms {
            truncated
        } else {
            let preserved = current.permissions & PermissionFlags::THREAD_GATED_BITS.bits();
            (truncated & !PermissionFlags::THREAD_GATED_BITS.bits()) | preserved
        };
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
        let validated = validate_asset_opt("icon", Some(val))?;
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

    let mut updated = Role::from(updated_row);
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

    if !can_write_thread_perms {
        updated.permissions &= !PermissionFlags::THREAD_GATED_BITS.bits();
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

fn validate_role_name(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_ROLE_NAME_LENGTH {
        return Err(AppError::validation(
            "name",
            ValidationCode::StringLengthInvalid,
            "Role name must be between 1 and 100 characters.",
        ));
    }
    reject_control_chars("name", trimmed)?;
    Ok(trimmed.to_string())
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
