use std::collections::{HashMap, HashSet};

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::MAX_GUILD_ROLES;
use crate::error::{AppError, ValidationCode};
use crate::models::gateway::events::GuildRoleBulkEvent;
use crate::models::rest::roles::{RoleHoistPositionUpdate, RolePositionUpdate};
use crate::models::role::{Role, RoleRow};
use crate::services::permission_service;

use super::crud::highest_member_role_position;

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
    if payload.len() > MAX_GUILD_ROLES as usize {
        return Err(AppError::validation(
            "roles",
            ValidationCode::InvalidFormat,
            "Too many role position updates.",
        ));
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
            && entry
                .position
                .is_some_and(|pos| pos != role.position as i32)
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
    if payload.len() > MAX_GUILD_ROLES as usize {
        return Err(AppError::validation(
            "roles",
            ValidationCode::InvalidFormat,
            "Too many role hoist position updates.",
        ));
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
            RolePositionUpdate {
                id: "r3".into(),
                position: Some(1),
            },
            RolePositionUpdate {
                id: "r2".into(),
                position: Some(2),
            },
            RolePositionUpdate {
                id: "r1".into(),
                position: Some(3),
            },
        ];
        let (updates, final_roles) = compute_reordered_positions(&rows, &full, true, u32::MAX);
        assert_eq!(updates, vec![("r3".into(), 1), ("r1".into(), 3)]);
        assert_eq!(
            final_roles
                .iter()
                .map(|r| (r.id.as_str(), r.position))
                .collect::<Vec<_>>(),
            vec![("r1", 3), ("r2", 2), ("r3", 1)]
        );

        let move_up = vec![RolePositionUpdate {
            id: "r1".into(),
            position: Some(3),
        }];
        let (updates, _) = compute_reordered_positions(&rows, &move_up, true, u32::MAX);
        assert_eq!(
            updates,
            vec![("r2".into(), 1), ("r3".into(), 2), ("r1".into(), 3)]
        );

        let bounded = vec![RolePositionUpdate {
            id: "r1".into(),
            position: Some(2),
        }];
        let (updates, _) = compute_reordered_positions(&rows, &bounded, false, 3);
        assert_eq!(updates, vec![("r2".into(), 1), ("r1".into(), 2)]);
    }
}
