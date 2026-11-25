use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::Serialize;
use sqlx::{MySql, MySqlConnection, QueryBuilder};
use tracing::error;

use crate::AppState;
use crate::constants::channel::{ChannelType, OverwriteType};
use crate::constants::flags::PermissionFlags;
use crate::error::AppError;
use crate::gateway::session::GatewaySession;
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, PermissionOverwrite,
};
use crate::models::guild_member::GuildMemberRole;
use crate::models::role::RolePerm;

#[derive(Debug, Clone)]
pub struct ChannelPermissionContext {
    pub owner_id: String,
    pub roles: Vec<RolePerm>,
    pub overwrites: Vec<PermissionOverwrite>,
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

fn apply_channel_overwrites(
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

    perms
}

pub fn compute_channel_permissions(
    base: PermissionFlags,
    guild_id: &str,
    user_id: &str,
    member_role_ids: &[String],
    overwrites: &[PermissionOverwrite],
) -> PermissionFlags {
    let mut perms =
        apply_channel_overwrites(base, guild_id, user_id, member_role_ids, overwrites);
    if perms.is_empty() {
        return perms;
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

pub fn compute_thread_permissions(
    base: PermissionFlags,
    guild_id: &str,
    user_id: &str,
    member_role_ids: &[String],
    parent_overwrites: &[PermissionOverwrite],
) -> PermissionFlags {
    let mut perms =
        apply_channel_overwrites(base, guild_id, user_id, member_role_ids, parent_overwrites);
    if perms.is_empty() {
        return perms;
    }
    if !perms.contains(PermissionFlags::SEND_MESSAGES_IN_THREADS) {
        perms.remove(
            PermissionFlags::MENTION_EVERYONE
                | PermissionFlags::SEND_TTS_MESSAGES
                | PermissionFlags::ATTACH_FILES
                | PermissionFlags::EMBED_LINKS,
        );
    }
    perms
}

async fn load_guild_role_context_conn(
    conn: &mut MySqlConnection,
    guild_id: &str,
    user_id: &str,
) -> Result<(String, Vec<RolePerm>, Vec<String>), AppError> {
    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    if owner_id == user_id {
        return Ok((owner_id, Vec::new(), Vec::new()));
    }

    let is_member: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM guild_members WHERE guild_id = ? AND user_id = ? LIMIT 1")
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;

    if is_member.is_none() {
        return Err(AppError::missing_permissions());
    }

    let roles =
        sqlx::query_as::<_, RolePerm>("SELECT id, permissions FROM roles WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_all(&mut *conn)
            .await?;

    let member_roles: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(guild_id)
    .fetch_all(&mut *conn)
    .await?;

    Ok((owner_id, roles, member_roles))
}

pub async fn compute_guild_permissions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<PermissionFlags, AppError> {
    let mut conn = state.db.acquire().await?;
    let (owner_id, roles, member_roles) =
        load_guild_role_context_conn(&mut conn, guild_id, user_id).await?;
    Ok(compute_base_permissions(
        &owner_id,
        guild_id,
        user_id,
        &roles,
        &member_roles,
    ))
}

pub async fn compute_user_channel_permissions_conn(
    conn: &mut MySqlConnection,
    guild_id: &str,
    channel_id: &str,
    user_id: &str,
) -> Result<PermissionFlags, AppError> {
    let (owner_id, roles, member_roles) =
        load_guild_role_context_conn(conn, guild_id, user_id).await?;
    let base = compute_base_permissions(&owner_id, guild_id, user_id, &roles, &member_roles);
    if base.contains(PermissionFlags::ADMINISTRATOR) {
        return Ok(PermissionFlags::all());
    }

    let overwrites: Vec<PermissionOverwrite> = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(conn)
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

pub async fn compute_user_thread_permissions_conn(
    conn: &mut MySqlConnection,
    guild_id: &str,
    parent_id: &str,
    user_id: &str,
) -> Result<(PermissionFlags, PermissionFlags), AppError> {
    let (owner_id, roles, member_roles) =
        load_guild_role_context_conn(conn, guild_id, user_id).await?;
    let base = compute_base_permissions(&owner_id, guild_id, user_id, &roles, &member_roles);
    if base.contains(PermissionFlags::ADMINISTRATOR) {
        return Ok((PermissionFlags::all(), PermissionFlags::all()));
    }

    let overwrites: Vec<PermissionOverwrite> = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(parent_id)
    .fetch_all(conn)
    .await?
    .into_iter()
    .map(PermissionOverwrite::from)
    .collect();

    let parent_perms =
        compute_channel_permissions(base, guild_id, user_id, &member_roles, &overwrites);
    let thread_perms =
        compute_thread_permissions(base, guild_id, user_id, &member_roles, &overwrites);
    Ok((parent_perms, thread_perms))
}

pub async fn compute_user_channel_permissions(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    user_id: &str,
) -> Result<PermissionFlags, AppError> {
    let mut conn = state.db.acquire().await?;
    compute_user_channel_permissions_conn(&mut conn, guild_id, channel_id, user_id).await
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

pub async fn require_channel_permission_with_ctx(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    user_id: &str,
    required: PermissionFlags,
) -> Result<
    (
        Option<ChannelPermissionContext>,
        Option<Vec<String>>,
        PermissionFlags,
    ),
    AppError,
> {
    let mut conn = state.db.acquire().await?;
    let (owner_id, roles, member_roles) =
        load_guild_role_context_conn(&mut conn, guild_id, user_id).await?;
    if owner_id == user_id {
        return Ok((None, None, PermissionFlags::all()));
    }

    let base = compute_base_permissions(&owner_id, guild_id, user_id, &roles, &member_roles);
    if base.contains(PermissionFlags::ADMINISTRATOR) {
        return Ok((None, Some(member_roles), PermissionFlags::all()));
    }

    let overwrites: Vec<PermissionOverwrite> = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(&mut *conn)
    .await?
    .into_iter()
    .map(PermissionOverwrite::from)
    .collect();

    let perms = compute_channel_permissions(base, guild_id, user_id, &member_roles, &overwrites);
    if !perms.contains(required) {
        return Err(AppError::missing_permissions());
    }

    let ctx = ChannelPermissionContext {
        owner_id,
        roles,
        overwrites,
    };
    Ok((Some(ctx), Some(member_roles), perms))
}

pub fn filter_visible_channels_with_context(
    owner_id: &str,
    guild_id: &str,
    user_id: &str,
    roles: &[RolePerm],
    member_roles: &[String],
    channels: Vec<ChannelResponse>,
) -> Vec<ChannelResponse> {
    if channels.is_empty() {
        return channels;
    }

    let base = compute_base_permissions(owner_id, guild_id, user_id, roles, member_roles);
    if base.contains(PermissionFlags::ADMINISTRATOR) {
        return channels;
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
                member_roles,
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
                    member_roles,
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

    channels
        .into_iter()
        .filter(|ch| visible_ids.contains(&ch.base.id))
        .collect()
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

    let mut conn = state.db.acquire().await?;
    let (owner_id, roles, member_roles) =
        load_guild_role_context_conn(&mut conn, guild_id, user_id).await?;
    Ok(filter_visible_channels_with_context(
        &owner_id,
        guild_id,
        user_id,
        &roles,
        &member_roles,
        channels,
    ))
}

pub async fn filter_sessions_for_channel_with_ctx(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    ctx: Option<&ChannelPermissionContext>,
) -> Vec<Arc<GatewaySession>> {
    let sessions = state.hub.sessions_for_guild(guild_id);
    if sessions.is_empty() {
        return sessions;
    }

    let owned_ctx;
    let ctx = match ctx {
        Some(c) => c,
        None => {
            let owner_id = match sqlx::query_scalar::<_, String>(
                "SELECT owner_id FROM guilds WHERE id = ?",
            )
            .bind(guild_id)
            .fetch_optional(&state.db)
            .await
            {
                Ok(Some(id)) => id,
                Ok(None) => return Vec::new(),
                Err(err) => {
                    error!(error = %err, guild_id = %guild_id, "Failed to load guild owner for channel dispatch");
                    return Vec::new();
                }
            };

            let roles: Vec<RolePerm> = match sqlx::query_as(
                "SELECT id, permissions FROM roles WHERE guild_id = ?",
            )
            .bind(guild_id)
            .fetch_all(&state.db)
            .await
            {
                Ok(r) => r,
                Err(err) => {
                    error!(error = %err, guild_id = %guild_id, "Failed to load guild roles for channel dispatch");
                    return Vec::new();
                }
            };

            let overwrites: Vec<PermissionOverwrite> = match sqlx::query_as::<_, ChannelOverwriteRow>(
                "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
            )
            .bind(channel_id)
            .fetch_all(&state.db)
            .await
            {
                Ok(rows) => rows.into_iter().map(PermissionOverwrite::from).collect(),
                Err(err) => {
                    error!(error = %err, channel_id = %channel_id, "Failed to load channel overwrites for channel dispatch");
                    return Vec::new();
                }
            };

            owned_ctx = ChannelPermissionContext {
                owner_id,
                roles,
                overwrites,
            };
            &owned_ctx
        }
    };

    let user_ids: Vec<String> = sessions
        .iter()
        .filter_map(|s| s.user_id())
        .filter(|uid| uid != &ctx.owner_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut roles_by_user: HashMap<String, Vec<String>> = HashMap::new();
    if !user_ids.is_empty() {
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

        let role_rows: Vec<GuildMemberRole> = match qb.build_query_as().fetch_all(&state.db).await {
            Ok(rows) => rows,
            Err(err) => {
                error!(error = %err, guild_id = %guild_id, "Failed to load member roles for channel dispatch");
                return Vec::new();
            }
        };

        for r in role_rows {
            roles_by_user.entry(r.user_id).or_default().push(r.role_id);
        }
    }

    let mut can_view: HashMap<String, bool> = HashMap::with_capacity(user_ids.len() + 1);
    can_view.insert(ctx.owner_id.clone(), true);
    for uid in user_ids {
        let member_roles = roles_by_user.get(&uid).map(Vec::as_slice).unwrap_or(&[]);
        let base =
            compute_base_permissions(&ctx.owner_id, guild_id, &uid, &ctx.roles, member_roles);
        let ch_perms =
            compute_channel_permissions(base, guild_id, &uid, member_roles, &ctx.overwrites);
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

pub async fn dispatch_channel_with_ctx<T: Serialize>(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    ctx: Option<&ChannelPermissionContext>,
    event_name: &str,
    data: &T,
) {
    let mut sessions =
        filter_sessions_for_channel_with_ctx(state, guild_id, channel_id, ctx).await;
    if event_name.starts_with("THREAD_") {
        sessions.retain(|s| s.has_channel_threads());
    }
    state.dispatch_to_sessions(&sessions, event_name, data);
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

        let admin_base =
            compute_base_permissions("u_owner", "g1", "u_admin", &roles, &["r_admin".to_string()]);
        let deny_all = vec![ow_role(
            "g1",
            PermissionFlags::empty(),
            PermissionFlags::VIEW_CHANNEL,
        )];
        assert_eq!(
            compute_channel_permissions(
                admin_base,
                "g1",
                "u_admin",
                &["r_admin".to_string()],
                &deny_all
            ),
            PermissionFlags::all()
        );
    }

    #[test]
    fn overwrite_precedence_and_implicit_denies() {
        let base = PermissionFlags::VIEW_CHANNEL
            | PermissionFlags::SEND_MESSAGES
            | PermissionFlags::ATTACH_FILES
            | PermissionFlags::EMBED_LINKS
            | PermissionFlags::MENTION_EVERYONE
            | PermissionFlags::SEND_TTS_MESSAGES;
        let roles = vec!["r1".to_string(), "r2".to_string()];

        let overwrites = vec![
            ow_role(
                "g1",
                PermissionFlags::empty(),
                PermissionFlags::VIEW_CHANNEL,
            ),
            ow_role(
                "r1",
                PermissionFlags::empty(),
                PermissionFlags::SEND_MESSAGES,
            ),
            ow_role(
                "r2",
                PermissionFlags::VIEW_CHANNEL,
                PermissionFlags::empty(),
            ),
            ow_member(
                "u1",
                PermissionFlags::SEND_MESSAGES,
                PermissionFlags::empty(),
            ),
        ];

        let perms = compute_channel_permissions(base, "g1", "u1", &roles, &overwrites);
        assert!(perms.contains(PermissionFlags::VIEW_CHANNEL));
        assert!(perms.contains(PermissionFlags::SEND_MESSAGES));
        assert!(perms.contains(PermissionFlags::ATTACH_FILES));

        let perms_no_member =
            compute_channel_permissions(base, "g1", "u1", &roles, &overwrites[..3]);
        assert!(perms_no_member.contains(PermissionFlags::VIEW_CHANNEL));
        assert!(!perms_no_member.contains(PermissionFlags::SEND_MESSAGES));
        assert!(!perms_no_member.contains(PermissionFlags::ATTACH_FILES));
        assert!(!perms_no_member.contains(PermissionFlags::EMBED_LINKS));
        assert!(!perms_no_member.contains(PermissionFlags::MENTION_EVERYONE));
        assert!(!perms_no_member.contains(PermissionFlags::SEND_TTS_MESSAGES));

        let hidden = compute_channel_permissions(base, "g1", "u1", &[], &overwrites[..1]);
        assert!(hidden.is_empty());
    }

    #[test]
    fn role_allow_overrides_other_role_deny_and_member_deny_wins() {
        let base = PermissionFlags::VIEW_CHANNEL | PermissionFlags::SEND_MESSAGES;
        let roles = vec!["r1".to_string(), "r2".to_string()];
        let role_conflict = vec![
            ow_role(
                "r1",
                PermissionFlags::empty(),
                PermissionFlags::SEND_MESSAGES,
            ),
            ow_role(
                "r2",
                PermissionFlags::SEND_MESSAGES,
                PermissionFlags::empty(),
            ),
        ];

        let perms = compute_channel_permissions(base, "g1", "u1", &roles, &role_conflict);
        assert!(perms.contains(PermissionFlags::SEND_MESSAGES));

        let with_member_deny = vec![
            ow_role(
                "r2",
                PermissionFlags::SEND_MESSAGES,
                PermissionFlags::empty(),
            ),
            ow_member(
                "u1",
                PermissionFlags::empty(),
                PermissionFlags::SEND_MESSAGES,
            ),
        ];
        let perms2 = compute_channel_permissions(base, "g1", "u1", &roles, &with_member_deny);
        assert!(perms2.contains(PermissionFlags::VIEW_CHANNEL));
        assert!(!perms2.contains(PermissionFlags::SEND_MESSAGES));
    }

    #[test]
    fn category_visibility_depends_on_visible_children() {
        use crate::models::channel::GuildChannel;

        let roles = vec![RolePerm {
            id: "g1".to_string(),
            permissions: PermissionFlags::VIEW_CHANNEL.bits(),
        }];
        let make_ch = |id: &str, kind: ChannelType, parent: Option<&str>, ows| {
            ChannelResponse::from_parts(
                GuildChannel {
                    id: id.to_string(),
                    kind: kind as u32,
                    position: 0,
                    guild_id: "g1".to_string(),
                    parent_id: parent.map(str::to_string),
                    name: id.to_string(),
                    last_message_id: None,
                },
                ows,
            )
        };
        let deny_view = vec![ow_role(
            "g1",
            PermissionFlags::empty(),
            PermissionFlags::VIEW_CHANNEL,
        )];

        let channels = vec![
            make_ch("cat_hidden", ChannelType::GuildCategory, None, vec![]),
            make_ch(
                "ch_secret",
                ChannelType::GuildText,
                Some("cat_hidden"),
                deny_view.clone(),
            ),
            make_ch("cat_empty", ChannelType::GuildCategory, None, vec![]),
            make_ch("cat_mixed", ChannelType::GuildCategory, None, vec![]),
            make_ch(
                "ch_public",
                ChannelType::GuildText,
                Some("cat_mixed"),
                vec![],
            ),
        ];

        let visible: Vec<String> =
            filter_visible_channels_with_context("owner", "g1", "u1", &roles, &[], channels)
                .into_iter()
                .map(|c| c.base.id)
                .collect();

        assert_eq!(visible, vec!["cat_empty", "cat_mixed", "ch_public"]);
    }
}
