use std::collections::{HashMap, HashSet};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::{MAX_CHANNELS_PER_CATEGORY, MAX_CHANNELS_PER_GUILD};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::{ChannelResponse, ChannelRow};
use crate::models::rest::channels::ChannelPositionUpdate;
use crate::services::{permission_service, queries};

use super::forum::fetch_guild_forum_tags;
use super::overwrites::{check_overwrite_delta_permissions, replace_channel_overwrites};

pub async fn modify_guild_channel_positions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: Vec<ChannelPositionUpdate>,
) -> Result<(), AppError> {
    let perms = permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !perms.contains(PermissionFlags::MANAGE_CHANNELS) {
        return Err(AppError::missing_permissions());
    }

    if payload.is_empty() {
        return Ok(());
    }
    if payload.len() > MAX_CHANNELS_PER_GUILD as usize {
        return Err(AppError::validation(
            "channels",
            ValidationCode::InvalidFormat,
            "Too many channel position updates.",
        ));
    }

    let mut tx = state.db.begin().await?;

    let rows = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE guild_id = ? AND type NOT IN (10, 11, 12) ORDER BY position ASC, CAST(id AS UNSIGNED) ASC FOR UPDATE",
        ChannelRow::SELECT_COLS
    ))
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut by_id: HashMap<String, ChannelRow> =
        rows.into_iter().map(|r| (r.id.clone(), r)).collect();

    for entry in &payload {
        let Some(ch) = by_id.get(&entry.id) else {
            return Err(AppError::validation(
                "id",
                ValidationCode::ChannelNotFound,
                "Channel does not exist in this community.",
            ));
        };
        if let Some(Some(ref pid)) = entry.parent_id {
            if ch.kind == ChannelType::GuildCategory as u32 {
                return Err(AppError::validation(
                    "parent_id",
                    ValidationCode::CategoriesCannotHaveParents,
                    "Categories cannot have a parent category.",
                ));
            }
            let Some(parent) = by_id.get(pid) else {
                return Err(AppError::validation(
                    "parent_id",
                    ValidationCode::InvalidParentChannel,
                    "Parent channel does not exist in this community.",
                ));
            };
            if parent.kind != ChannelType::GuildCategory as u32 {
                return Err(AppError::validation(
                    "parent_id",
                    ValidationCode::ParentMustBeCategory,
                    "Parent channel must be a category.",
                ));
            }
        }
        if let Some(Some(ref sib_id)) = entry.preceding_sibling_id {
            if sib_id == &entry.id {
                return Err(AppError::validation(
                    "preceding_sibling_id",
                    ValidationCode::CannotPositionChannelRelativeToItself,
                    "Cannot position a channel relative to itself.",
                ));
            }
            if !by_id.contains_key(sib_id) {
                return Err(AppError::validation(
                    "preceding_sibling_id",
                    ValidationCode::ChannelNotFound,
                    "Preceding channel does not exist in this community.",
                ));
            }
        }
    }

    let original_state: HashMap<String, (Option<String>, u32)> = by_id
        .iter()
        .map(|(id, ch)| (id.clone(), (ch.parent_id.clone(), ch.position)))
        .collect();

    let mut lock_copies: Vec<(String, String)> = Vec::new();
    let mut dirty_ids: HashSet<String> = HashSet::new();

    for entry in payload {
        let Some(current_ch) = by_id.get(&entry.id).cloned() else {
            continue;
        };
        let old_parent = current_ch.parent_id.clone();
        let new_parent = match entry.parent_id {
            Some(opt) => opt.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
            None => old_parent.clone(),
        };

        if let Some(Some(ref sib_id)) = entry.preceding_sibling_id
            && by_id
                .get(sib_id)
                .is_none_or(|sib| sib.parent_id != new_parent)
        {
            return Err(AppError::validation(
                "preceding_sibling_id",
                ValidationCode::PrecedingChannelMustShareParent,
                "Preceding sibling must share the destination parent.",
            ));
        }

        if new_parent != old_parent
            && let Some(ref pid) = new_parent
        {
            let count = by_id
                .values()
                .filter(|c| c.id != entry.id && c.parent_id.as_deref() == Some(pid.as_str()))
                .count() as u64;
            if count >= MAX_CHANNELS_PER_CATEGORY {
                return Err(AppError::max_category_channels());
            }
            if entry.lock_permissions == Some(true) {
                lock_copies.push((entry.id.clone(), pid.clone()));
            }
        }

        if let Some(ch) = by_id.get_mut(&entry.id) {
            ch.parent_id = new_parent.clone();
        }

        let mut siblings: Vec<String> = by_id
            .values()
            .filter(|c| c.id != entry.id && c.parent_id == new_parent)
            .map(|c| c.id.clone())
            .collect();
        siblings.sort_by_key(|id| by_id.get(id).map_or(0, |c| c.position));

        let insert_idx = if let Some(sib_opt) = entry.preceding_sibling_id {
            match sib_opt {
                None => 0,
                Some(sib_id) => siblings
                    .iter()
                    .position(|id| id == &sib_id)
                    .map_or(siblings.len(), |idx| idx + 1),
            }
        } else if let Some(pos) = entry.position {
            (pos as usize).min(siblings.len())
        } else if current_ch.kind == ChannelType::GuildVoice as u32 {
            siblings.len()
        } else {
            siblings
                .iter()
                .position(|id| {
                    by_id
                        .get(id)
                        .is_some_and(|c| c.kind == ChannelType::GuildVoice as u32)
                })
                .unwrap_or(siblings.len())
        };

        siblings.insert(insert_idx, entry.id.clone());
        for (idx, sib_id) in siblings.into_iter().enumerate() {
            if let Some(ch) = by_id.get_mut(&sib_id) {
                ch.position = idx as u32;
            }
        }
    }

    for (cid, (orig_parent, orig_pos)) in &original_state {
        if let Some(ch) = by_id.get(cid)
            && (&ch.parent_id != orig_parent || ch.position != *orig_pos)
        {
            sqlx::query("UPDATE channels SET parent_id = ?, position = ? WHERE id = ?")
                .bind(&ch.parent_id)
                .bind(ch.position)
                .bind(cid)
                .execute(&mut *tx)
                .await?;
            dirty_ids.insert(cid.clone());
        }
    }

    if !lock_copies.is_empty() {
        let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
            .bind(guild_id)
            .fetch_one(&mut *tx)
            .await?;
        for (cid, pid) in lock_copies {
            let ch_perms = permission_service::compute_user_channel_permissions_conn(
                &mut tx, guild_id, &cid, user_id,
            )
            .await?;
            if !ch_perms.contains(PermissionFlags::MANAGE_ROLES) {
                return Err(AppError::missing_permissions());
            }
            let parent_ows = queries::fetch_channel_overwrites(&mut *tx, &pid).await?;
            let old_ows = queries::fetch_channel_overwrites(&mut *tx, &cid).await?;
            let is_owner_or_admin =
                user_id == owner_id || ch_perms.contains(PermissionFlags::ADMINISTRATOR);
            if !is_owner_or_admin {
                check_overwrite_delta_permissions(ch_perms, &old_ows, &parent_ows)?;
            }
            replace_channel_overwrites(&mut tx, &cid, &parent_ows).await?;
            dirty_ids.insert(cid);
        }
    }

    let mut tags_by_ch = fetch_guild_forum_tags(&mut *tx, guild_id).await?;
    let mut updated_channels = Vec::with_capacity(dirty_ids.len());
    for cid in dirty_ids {
        if let Some(row) = by_id.remove(&cid) {
            let ows = queries::fetch_channel_overwrites(&mut *tx, &cid).await?;
            let tags = tags_by_ch.remove(&cid).unwrap_or_default();
            updated_channels.push(ChannelResponse::from_row(row, ows, tags));
        }
    }

    tx.commit().await?;

    for ch in updated_channels {
        permission_service::dispatch_channel_with_ctx(
            state,
            guild_id,
            &ch.base.id,
            None,
            events::CHANNEL_UPDATE,
            &ch,
        )
        .await;
    }

    Ok(())
}
