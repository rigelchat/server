use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sqlx::MySql;

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::MAX_CHANNEL_OVERWRITES;
use crate::error::{AppError, ValidationCode};
use crate::gateway::session::GatewaySession;
use crate::models::channel::{ChannelResponse, ChannelRow, ChannelType, PermissionOverwrite};
use crate::models::permission::OverwriteType;
use crate::models::rest::channels::{PermissionOverwriteInput, SetPermissionOverwriteRequest};
use crate::services::{permission_service, queries};
use crate::utils::snowflake::validate_snowflake;

pub async fn set_permission_overwrite(
    state: &AppState,
    channel_id: &str,
    overwrite_id: &str,
    user_id: &str,
    payload: SetPermissionOverwriteRequest,
) -> Result<(), AppError> {
    let current = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    let (perm_ctx, _, actor_perms) = permission_service::require_channel_permission_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_ROLES,
    )
    .await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(&current.guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let is_owner_or_admin =
        user_id == owner_id || actor_perms.contains(PermissionFlags::ADMINISTRATOR);

    validate_snowflake("overwrite_id", overwrite_id)?;

    let mut tx = state.db.begin().await?;
    let old_overwrites = queries::fetch_channel_overwrites(&mut *tx, channel_id).await?;

    let mut next_inputs: Vec<PermissionOverwriteInput> = old_overwrites
        .iter()
        .filter(|o| o.id != overwrite_id)
        .map(|o| PermissionOverwriteInput {
            id: o.id.clone(),
            kind: o.kind,
            allow: Some(o.allow),
            deny: Some(o.deny),
        })
        .collect();
    next_inputs.push(PermissionOverwriteInput {
        id: overwrite_id.to_string(),
        kind: payload.kind,
        allow: payload.allow,
        deny: payload.deny,
    });

    let new_overwrites = validate_and_normalize_overwrites(
        &mut tx,
        &current.guild_id,
        &next_inputs,
        actor_perms,
        is_owner_or_admin,
        Some(&old_overwrites),
    )
    .await?;

    let old_sessions = permission_service::filter_sessions_for_channel_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        perm_ctx.as_ref(),
    )
    .await;

    replace_channel_overwrites(&mut tx, channel_id, &new_overwrites).await?;

    let synced_children = if current.kind == ChannelType::GuildCategory as u32 {
        propagate_category_overwrites(
            &mut tx,
            &current.guild_id,
            channel_id,
            &old_overwrites,
            &new_overwrites,
        )
        .await?
    } else {
        Vec::new()
    };

    tx.commit().await?;

    let guild_id = current.guild_id.clone();
    let updated = ChannelResponse::from_row(current, new_overwrites);
    dispatch_channel_update_with_visibility(state, &guild_id, channel_id, old_sessions, &updated)
        .await;

    for child in synced_children {
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            &child.base.id,
            None,
            events::CHANNEL_UPDATE,
            &child,
        )
        .await;
    }

    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, &guild_id).await;

    Ok(())
}

pub async fn delete_permission_overwrite(
    state: &AppState,
    channel_id: &str,
    overwrite_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let current = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    let (perm_ctx, _, actor_perms) = permission_service::require_channel_permission_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_ROLES,
    )
    .await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(&current.guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let is_owner_or_admin =
        user_id == owner_id || actor_perms.contains(PermissionFlags::ADMINISTRATOR);

    let mut tx = state.db.begin().await?;
    let old_overwrites = queries::fetch_channel_overwrites(&mut *tx, channel_id).await?;

    let Some(target_ow) = old_overwrites.iter().find(|o| o.id == overwrite_id) else {
        tx.commit().await?;
        return Ok(());
    };

    if !is_owner_or_admin && (target_ow.deny & !actor_perms.bits()) != 0 {
        return Err(AppError::missing_permissions());
    }

    let new_overwrites: Vec<PermissionOverwrite> = old_overwrites
        .iter()
        .filter(|o| o.id != overwrite_id)
        .cloned()
        .collect();

    let old_sessions = permission_service::filter_sessions_for_channel_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        perm_ctx.as_ref(),
    )
    .await;

    sqlx::query("DELETE FROM channel_permission_overwrites WHERE channel_id = ? AND target_id = ?")
        .bind(channel_id)
        .bind(overwrite_id)
        .execute(&mut *tx)
        .await?;

    let synced_children = if current.kind == ChannelType::GuildCategory as u32 {
        propagate_category_overwrites(
            &mut tx,
            &current.guild_id,
            channel_id,
            &old_overwrites,
            &new_overwrites,
        )
        .await?
    } else {
        Vec::new()
    };

    tx.commit().await?;

    let guild_id = current.guild_id.clone();
    let updated = ChannelResponse::from_row(current, new_overwrites);
    dispatch_channel_update_with_visibility(state, &guild_id, channel_id, old_sessions, &updated)
        .await;

    for child in synced_children {
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            &child.base.id,
            None,
            events::CHANNEL_UPDATE,
            &child,
        )
        .await;
    }

    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, &guild_id).await;

    Ok(())
}

pub(super) async fn replace_channel_overwrites(
    tx: &mut sqlx::Transaction<'_, MySql>,
    channel_id: &str,
    overwrites: &[PermissionOverwrite],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM channel_permission_overwrites WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut **tx)
        .await?;

    for ow in overwrites {
        sqlx::query(
            "INSERT INTO channel_permission_overwrites (type, channel_id, target_id, allow, deny) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(ow.kind)
        .bind(channel_id)
        .bind(&ow.id)
        .bind(ow.allow)
        .bind(ow.deny)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

pub(super) async fn validate_and_normalize_overwrites(
    tx: &mut sqlx::Transaction<'_, MySql>,
    guild_id: &str,
    inputs: &[PermissionOverwriteInput],
    actor_perms: PermissionFlags,
    is_owner_or_admin: bool,
    old_overwrites: Option<&[PermissionOverwrite]>,
) -> Result<Vec<PermissionOverwrite>, AppError> {
    if inputs.len() > MAX_CHANNEL_OVERWRITES {
        return Err(AppError::validation(
            "permission_overwrites",
            ValidationCode::InvalidFormat,
            "Too many permission overwrites.",
        ));
    }

    let mut deduped: Vec<PermissionOverwrite> = Vec::with_capacity(inputs.len());
    let mut index_by_id: HashMap<String, usize> = HashMap::with_capacity(inputs.len());

    for input in inputs {
        if input.kind != OverwriteType::Role as u32 && input.kind != OverwriteType::Member as u32 {
            return Err(AppError::validation(
                "permission_overwrites",
                ValidationCode::InvalidFormat,
                "Invalid permission overwrite type.",
            ));
        }
        validate_snowflake("permission_overwrites", &input.id)?;

        let exists: Option<(i32,)> = if input.kind == OverwriteType::Role as u32 {
            sqlx::query_as("SELECT 1 FROM roles WHERE id = ? AND guild_id = ? LIMIT 1")
                .bind(&input.id)
                .bind(guild_id)
                .fetch_optional(&mut **tx)
                .await?
        } else {
            sqlx::query_as("SELECT 1 FROM guild_members WHERE user_id = ? AND guild_id = ? LIMIT 1")
                .bind(&input.id)
                .bind(guild_id)
                .fetch_optional(&mut **tx)
                .await?
        };

        if exists.is_none() {
            return Err(AppError::validation(
                "permission_overwrites",
                ValidationCode::InvalidFormat,
                "Overwrite target does not belong to this community.",
            ));
        }

        let allow = PermissionFlags::from_bits_truncate(input.allow.unwrap_or(0)).bits();
        let deny = PermissionFlags::from_bits_truncate(input.deny.unwrap_or(0)).bits();
        let ow = PermissionOverwrite {
            id: input.id.clone(),
            kind: input.kind,
            allow,
            deny,
        };

        if let Some(&idx) = index_by_id.get(&input.id) {
            deduped[idx] = ow;
        } else {
            index_by_id.insert(input.id.clone(), deduped.len());
            deduped.push(ow);
        }
    }

    if !is_owner_or_admin {
        let empty = Vec::new();
        check_overwrite_delta_permissions(actor_perms, old_overwrites.unwrap_or(&empty), &deduped)?;
    }

    Ok(deduped)
}

pub(super) fn check_overwrite_delta_permissions(
    actor_perms: PermissionFlags,
    old_overwrites: &[PermissionOverwrite],
    new_overwrites: &[PermissionOverwrite],
) -> Result<(), AppError> {
    let old_by_id: HashMap<&str, &PermissionOverwrite> =
        old_overwrites.iter().map(|o| (o.id.as_str(), o)).collect();
    let new_by_id: HashMap<&str, &PermissionOverwrite> =
        new_overwrites.iter().map(|o| (o.id.as_str(), o)).collect();

    for ow in new_overwrites {
        let prev_allow = old_by_id.get(ow.id.as_str()).map_or(0, |prev| prev.allow);
        let added_allows = ow.allow & !prev_allow;
        if (added_allows & !actor_perms.bits()) != 0 {
            return Err(AppError::missing_permissions());
        }
        if let Some(prev) = old_by_id.get(ow.id.as_str()) {
            let cleared_denies = prev.deny & !ow.deny;
            if (cleared_denies & !actor_perms.bits()) != 0 {
                return Err(AppError::missing_permissions());
            }
        }
    }

    for prev in old_overwrites {
        if !new_by_id.contains_key(prev.id.as_str()) && (prev.deny & !actor_perms.bits()) != 0 {
            return Err(AppError::missing_permissions());
        }
    }

    Ok(())
}

fn sorted_overwrite_key(ows: &[PermissionOverwrite]) -> Vec<(u32, String, u64, u64)> {
    let mut keys: Vec<(u32, String, u64, u64)> = ows
        .iter()
        .map(|o| (o.kind, o.id.clone(), o.allow, o.deny))
        .collect();
    keys.sort();
    keys
}

pub(super) async fn propagate_category_overwrites(
    tx: &mut sqlx::Transaction<'_, MySql>,
    guild_id: &str,
    category_id: &str,
    old_category_overwrites: &[PermissionOverwrite],
    new_category_overwrites: &[PermissionOverwrite],
) -> Result<Vec<ChannelResponse>, AppError> {
    let old_key = sorted_overwrite_key(old_category_overwrites);
    let children = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE guild_id = ? AND parent_id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(guild_id)
    .bind(category_id)
    .fetch_all(&mut **tx)
    .await?;

    let mut synced = Vec::new();
    for child in children {
        let child_ows = queries::fetch_channel_overwrites(&mut **tx, &child.id).await?;
        if sorted_overwrite_key(&child_ows) == old_key {
            replace_channel_overwrites(tx, &child.id, new_category_overwrites).await?;
            synced.push(ChannelResponse::from_row(
                child,
                new_category_overwrites.to_vec(),
            ));
        }
    }
    Ok(synced)
}

pub(super) async fn dispatch_channel_update_with_visibility(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    old_sessions: Vec<Arc<GatewaySession>>,
    updated: &ChannelResponse,
) {
    let new_sessions =
        permission_service::filter_sessions_for_channel_with_ctx(state, guild_id, channel_id, None)
            .await;

    let old_ids: HashSet<String> = old_sessions.iter().map(|s| s.id.clone()).collect();
    let new_ids: HashSet<String> = new_sessions.iter().map(|s| s.id.clone()).collect();

    let update_targets: Vec<Arc<GatewaySession>> = new_sessions
        .iter()
        .filter(|s| old_ids.contains(&s.id))
        .cloned()
        .collect();
    let create_targets: Vec<Arc<GatewaySession>> = new_sessions
        .into_iter()
        .filter(|s| !old_ids.contains(&s.id))
        .collect();
    let delete_targets: Vec<Arc<GatewaySession>> = old_sessions
        .into_iter()
        .filter(|s| !new_ids.contains(&s.id))
        .collect();

    state.dispatch_to_sessions(&update_targets, events::CHANNEL_UPDATE, updated);
    state.dispatch_to_sessions(&create_targets, events::CHANNEL_CREATE, updated);
    state.dispatch_to_sessions(&delete_targets, events::CHANNEL_DELETE, updated);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overwrite_delta_allows_untouched_existing_allows_and_blocks_escalation() {
        let actor = PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_ROLES;
        let existing = PermissionOverwrite {
            id: "r_admin".to_string(),
            kind: OverwriteType::Role as u32,
            allow: (PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_MESSAGES).bits(),
            deny: PermissionFlags::EMBED_LINKS.bits(),
        };
        let added_allowed = PermissionOverwrite {
            id: "r_mod".to_string(),
            kind: OverwriteType::Role as u32,
            allow: PermissionFlags::VIEW_CHANNEL.bits(),
            deny: 0,
        };
        assert!(
            check_overwrite_delta_permissions(
                actor,
                std::slice::from_ref(&existing),
                &[existing.clone(), added_allowed]
            )
            .is_ok()
        );

        let escalated = PermissionOverwrite {
            id: "r_mod".to_string(),
            kind: OverwriteType::Role as u32,
            allow: PermissionFlags::MANAGE_MESSAGES.bits(),
            deny: 0,
        };
        assert!(
            check_overwrite_delta_permissions(
                actor,
                std::slice::from_ref(&existing),
                &[existing.clone(), escalated]
            )
            .is_err()
        );

        let cleared_deny = PermissionOverwrite {
            id: "r_admin".to_string(),
            kind: OverwriteType::Role as u32,
            allow: existing.allow,
            deny: 0,
        };
        assert!(check_overwrite_delta_permissions(actor, &[existing], &[cleared_deny]).is_err());
    }
}
