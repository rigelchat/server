use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::error::AppError;
use crate::gateway::session::GatewaySession;
use crate::models::channel::{ChannelOverwriteRow, ChannelResponse, ChannelType, PermissionOverwrite};
use crate::models::guild_member::GuildMemberRole;
use crate::models::permission::OverwriteType;
use crate::services::member_service;

#[derive(Debug, Clone, FromRow)]
pub struct RolePerm {
    pub id: String,
    pub permissions: u64,
}

pub fn compute_base_permissions(
    owner_id: &str,
    guild_id: &str,
    user_id: &str,
    roles: &[RolePerm],
    member_role_ids: &[String],
) -> PermissionFlags {
    if user_id == owner_id {
        return PermissionFlags::all();
    }

    let everyone_perms = roles
        .iter()
        .find(|r| r.id == guild_id)
        .map(|r| r.permissions)
        .unwrap_or(0);

    let mut perms = PermissionFlags::from_bits_truncate(everyone_perms);
    let member_set: HashSet<&str> = member_role_ids.iter().map(String::as_str).collect();

    for role in roles {
        if member_set.contains(role.id.as_str()) {
            perms |= PermissionFlags::from_bits_truncate(role.permissions);
        }
    }

    if perms.contains(PermissionFlags::ADMINISTRATOR) {
        return PermissionFlags::all();
    }

    perms
}

pub fn compute_channel_permissions(
    base: PermissionFlags,
    guild_id: &str,
    user_id: &str,
    member_role_ids: &[String],
    overwrites: &[PermissionOverwrite],
) -> PermissionFlags {
    if base.contains(PermissionFlags::ADMINISTRATOR) {
        return PermissionFlags::all();
    }

    let mut perms = base;

    if let Some(ow) = overwrites
        .iter()
        .find(|o| o.kind == OverwriteType::Role as u32 && o.id == guild_id)
    {
        perms &= !PermissionFlags::from_bits_truncate(ow.deny);
        perms |= PermissionFlags::from_bits_truncate(ow.allow);
    }

    let member_set: HashSet<&str> = member_role_ids.iter().map(String::as_str).collect();
    let mut role_allow = PermissionFlags::empty();
    let mut role_deny = PermissionFlags::empty();

    for ow in overwrites {
        if ow.kind == OverwriteType::Role as u32
            && ow.id != guild_id
            && member_set.contains(ow.id.as_str())
        {
            role_allow |= PermissionFlags::from_bits_truncate(ow.allow);
            role_deny |= PermissionFlags::from_bits_truncate(ow.deny);
        }
    }

    perms &= !role_deny;
    perms |= role_allow;

    if let Some(ow) = overwrites
        .iter()
        .find(|o| o.kind == OverwriteType::Member as u32 && o.id == user_id)
    {
        perms &= !PermissionFlags::from_bits_truncate(ow.deny);
        perms |= PermissionFlags::from_bits_truncate(ow.allow);
    }

    if !perms.contains(PermissionFlags::VIEW_CHANNEL) {
        return PermissionFlags::empty();
    }

    if !perms.contains(PermissionFlags::SEND_MESSAGES) {
        perms.remove(
            PermissionFlags::MENTION_EVERYONE
                | PermissionFlags::SEND_TTS_MESSAGES
                | PermissionFlags::ATTACH_FILES
                | PermissionFlags::EMBED_LINKS,
        );
    }

    perms
}

async fn load_guild_role_context(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<(String, Vec<RolePerm>, Vec<String>), AppError> {
    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    if owner_id == user_id {
        return Ok((owner_id, Vec::new(), Vec::new()));
    }

    member_service::require_member(state, guild_id, user_id).await?;

    let roles = sqlx::query_as::<_, RolePerm>("SELECT id, permissions FROM roles WHERE guild_id = ?")
        .bind(guild_id)
        .fetch_all(&state.db)
        .await?;

    let member_roles: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    Ok((owner_id, roles, member_roles))
}

pub async fn compute_guild_permissions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<PermissionFlags, AppError> {
    let (owner_id, roles, member_roles) = load_guild_role_context(state, guild_id, user_id).await?;
    Ok(compute_base_permissions(
        &owner_id,
        guild_id,
        user_id,
        &roles,
        &member_roles,
    ))
}

pub async fn compute_user_channel_permissions(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    user_id: &str,
) -> Result<PermissionFlags, AppError> {
    let (owner_id, roles, member_roles) = load_guild_role_context(state, guild_id, user_id).await?;
    let base = compute_base_permissions(&owner_id, guild_id, user_id, &roles, &member_roles);
    if base.contains(PermissionFlags::ADMINISTRATOR) {
        return Ok(PermissionFlags::all());
    }

    let overwrites: Vec<PermissionOverwrite> = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(PermissionOverwrite::from)
    .collect();

    Ok(compute_channel_permissions(
        base,
        guild_id,
        user_id,
        &member_roles,
        &overwrites,
    ))
}

pub async fn require_channel_permission(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    user_id: &str,
    required: PermissionFlags,
) -> Result<PermissionFlags, AppError> {
    let perms = compute_user_channel_permissions(state, guild_id, channel_id, user_id).await?;
    if !perms.contains(required) {
        return Err(AppError::missing_permissions());
    }
    Ok(perms)
}

pub async fn filter_visible_channels(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    channels: Vec<ChannelResponse>,
) -> Result<Vec<ChannelResponse>, AppError> {
    if channels.is_empty() {
        return Ok(channels);
    }

    let (owner_id, roles, member_roles) = load_guild_role_context(state, guild_id, user_id).await?;
    let base = compute_base_permissions(&owner_id, guild_id, user_id, &roles, &member_roles);
    if base.contains(PermissionFlags::ADMINISTRATOR) {
        return Ok(channels);
    }

    let category_kind = ChannelType::GuildCategory as u32;
    let mut has_children: HashSet<String> = HashSet::new();
    let mut visible_parents: HashSet<String> = HashSet::new();
    let mut visible_ids: HashSet<String> = HashSet::new();

    for ch in &channels {
        if let Some(pid) = &ch.base.parent_id {
            has_children.insert(pid.clone());
        }
        if ch.base.kind != category_kind {
            let perms = compute_channel_permissions(
                base,
                guild_id,
                user_id,
                &member_roles,
                &ch.permission_overwrites,
            );
            if perms.contains(PermissionFlags::VIEW_CHANNEL) {
                visible_ids.insert(ch.base.id.clone());
                if let Some(pid) = &ch.base.parent_id {
                    visible_parents.insert(pid.clone());
                }
            }
        }
    }

    for ch in &channels {
        if ch.base.kind == category_kind {
            if visible_parents.contains(&ch.base.id) {
                visible_ids.insert(ch.base.id.clone());
            } else {
                let perms = compute_channel_permissions(
                    base,
                    guild_id,
                    user_id,
                    &member_roles,
                    &ch.permission_overwrites,
                );
                let empty_cat = !has_children.contains(&ch.base.id);
                if perms.contains(PermissionFlags::MANAGE_CHANNELS)
                    || (empty_cat && perms.contains(PermissionFlags::VIEW_CHANNEL))
                {
                    visible_ids.insert(ch.base.id.clone());
                }
            }
        }
    }

    Ok(channels
        .into_iter()
        .filter(|ch| visible_ids.contains(&ch.base.id))
        .collect())
}

pub async fn filter_sessions_for_channel(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
) -> Vec<Arc<GatewaySession>> {
    let sessions = state.hub.sessions_for_guild(guild_id);
    if sessions.is_empty() {
        return sessions;
    }

    let owner_id: Option<String> = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(owner_id) = owner_id else {
        return Vec::new();
    };

    let roles: Vec<RolePerm> =
        sqlx::query_as("SELECT id, permissions FROM roles WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

    let overwrites: Vec<PermissionOverwrite> = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(PermissionOverwrite::from)
    .collect();

    let user_ids: Vec<String> = sessions
        .iter()
        .filter_map(|s| s.user_id())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if user_ids.is_empty() {
        return Vec::new();
    }

    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT user_id, role_id FROM guild_member_roles WHERE guild_id = ",
    );
    qb.push_bind(guild_id);
    qb.push(" AND role_id != ");
    qb.push_bind(guild_id);
    qb.push(" AND user_id IN (");
    let mut sep = qb.separated(", ");
    for uid in &user_ids {
        sep.push_bind(uid);
    }
    qb.push(")");

    let role_rows: Vec<GuildMemberRole> =
        qb.build_query_as().fetch_all(&state.db).await.unwrap_or_default();

    let mut roles_by_user: HashMap<String, Vec<String>> = HashMap::new();
    for r in role_rows {
        roles_by_user.entry(r.user_id).or_default().push(r.role_id);
    }

    let mut can_view: HashMap<String, bool> = HashMap::with_capacity(user_ids.len());
    for uid in user_ids {
        let member_roles = roles_by_user.get(&uid).map(Vec::as_slice).unwrap_or(&[]);
        let base = compute_base_permissions(&owner_id, guild_id, &uid, &roles, member_roles);
        let ch_perms =
            compute_channel_permissions(base, guild_id, &uid, member_roles, &overwrites);
        can_view.insert(uid, ch_perms.contains(PermissionFlags::VIEW_CHANNEL));
    }

    sessions
        .into_iter()
        .filter(|s| {
            s.user_id()
                .is_some_and(|uid| can_view.get(&uid).copied().unwrap_or(false))
        })
        .collect()
}

pub async fn dispatch_channel<T: Serialize>(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    event_name: &str,
    data: &T,
) {
    let sessions = filter_sessions_for_channel(state, guild_id, channel_id).await;
    if sessions.is_empty() {
        return;
    }

    let value = match serde_json::to_value(data) {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(error = %err, event = %event_name, "Failed to serialize payload");
            return;
        }
    };

    for session in sessions {
        session.dispatch(event_name, value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ow_role(id: &str, allow: PermissionFlags, deny: PermissionFlags) -> PermissionOverwrite {
        PermissionOverwrite {
            id: id.to_string(),
            kind: OverwriteType::Role as u32,
            allow: allow.bits(),
            deny: deny.bits(),
        }
    }

    fn ow_member(id: &str, allow: PermissionFlags, deny: PermissionFlags) -> PermissionOverwrite {
        PermissionOverwrite {
            id: id.to_string(),
            kind: OverwriteType::Member as u32,
            allow: allow.bits(),
            deny: deny.bits(),
        }
    }

    #[test]
    fn owner_and_admin_bypass_overwrites() {
        let roles = vec![
            RolePerm {
                id: "g1".to_string(),
                permissions: PermissionFlags::VIEW_CHANNEL.bits(),
            },
            RolePerm {
                id: "r_admin".to_string(),
                permissions: PermissionFlags::ADMINISTRATOR.bits(),
            },
        ];

        assert_eq!(
            compute_base_permissions("u_owner", "g1", "u_owner", &roles, &[]),
            PermissionFlags::all()
        );

        let admin_base = compute_base_permissions(
            "u_owner",
            "g1",
            "u_admin",
            &roles,
            &["r_admin".to_string()],
        );
        let deny_all = vec![ow_role("g1", PermissionFlags::empty(), PermissionFlags::VIEW_CHANNEL)];
        assert_eq!(
            compute_channel_permissions(admin_base, "g1", "u_admin", &["r_admin".to_string()], &deny_all),
            PermissionFlags::all()
        );
    }

    #[test]
    fn overwrite_precedence_and_implicit_denies() {
        let base = PermissionFlags::VIEW_CHANNEL
            | PermissionFlags::SEND_MESSAGES
            | PermissionFlags::ATTACH_FILES
            | PermissionFlags::EMBED_LINKS;
        let roles = vec!["r1".to_string(), "r2".to_string()];

        // @everyone denies VIEW_CHANNEL, r1 denies SEND_MESSAGES, r2 allows VIEW_CHANNEL, member allows SEND_MESSAGES
        let overwrites = vec![
            ow_role("g1", PermissionFlags::empty(), PermissionFlags::VIEW_CHANNEL),
            ow_role("r1", PermissionFlags::empty(), PermissionFlags::SEND_MESSAGES),
            ow_role("r2", PermissionFlags::VIEW_CHANNEL, PermissionFlags::empty()),
            ow_member("u1", PermissionFlags::SEND_MESSAGES, PermissionFlags::empty()),
        ];

        let perms = compute_channel_permissions(base, "g1", "u1", &roles, &overwrites);
        assert!(perms.contains(PermissionFlags::VIEW_CHANNEL));
        assert!(perms.contains(PermissionFlags::SEND_MESSAGES));
        assert!(perms.contains(PermissionFlags::ATTACH_FILES));

        // Without member overwrite, r1 denies SEND_MESSAGES -> implicitly drops ATTACH_FILES & EMBED_LINKS
        let perms_no_member =
            compute_channel_permissions(base, "g1", "u1", &roles, &overwrites[..3]);
        assert!(perms_no_member.contains(PermissionFlags::VIEW_CHANNEL));
        assert!(!perms_no_member.contains(PermissionFlags::SEND_MESSAGES));
        assert!(!perms_no_member.contains(PermissionFlags::ATTACH_FILES));
        assert!(!perms_no_member.contains(PermissionFlags::EMBED_LINKS));

        // Denying VIEW_CHANNEL removes all permissions
        let hidden = compute_channel_permissions(base, "g1", "u1", &[], &overwrites[..1]);
        assert!(hidden.is_empty());
    }
}