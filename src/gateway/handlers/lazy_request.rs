use std::collections::{HashMap, HashSet};

use tracing::error;

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::{MEMBER_LIST_RANGE_MAX_SPAN, events};
use crate::constants::limits::MAX_LAZY_SUBSCRIBED_MEMBERS;
use crate::models::channel::{ChannelOverwriteRow, PermissionOverwrite};
use crate::models::gateway::lazy_request::{
    GuildMemberListUpdate, LazyListAction, LazyRequestPayload, MemberListGroup, MemberListItem,
    MemberListMember, MemberListPresence, MemberListSyncOp,
};
use crate::models::guild_member::{GuildMemberFields, GuildMemberRole, GuildMemberUserRow};
use crate::models::role::RolePerm;
use crate::models::user::{CustomStatus, PartialUser};
use crate::services::permission_service;

use super::presence;

struct SnapshotMember {
    user: PartialUser,
    nick: Option<String>,
    roles: Vec<String>,
    joined_at: u64,
    status: &'static str,
    mobile: bool,
    afk: bool,
    custom_status: Option<CustomStatus>,
}

struct GuildMemberListSnapshot {
    online_members: Vec<SnapshotMember>,
    offline_members: Vec<SnapshotMember>,
    viewer_allowed: HashSet<String>,
}

impl GuildMemberListSnapshot {
    fn build_update(
        &self,
        guild_id: &str,
        channel_id: &str,
        range: [i64; 2],
    ) -> GuildMemberListUpdate {
        let to_item = |m: &SnapshotMember| -> MemberListItem {
            let presence = MemberListPresence {
                status: m.status.to_string(),
                mobile: m.mobile,
                afk: m.afk,
                custom_status: m.custom_status.clone(),
            };

            MemberListItem::Member {
                member: MemberListMember {
                    user: m.user.clone(),
                    member: GuildMemberFields::new(m.nick.clone(), m.roles.clone(), m.joined_at),
                    presence,
                },
            }
        };

        let online_count = self.online_members.len();
        let offline_count = self.offline_members.len();
        let total_member_count = online_count + offline_count;

        let mut groups = Vec::new();
        let mut online_group = None;
        let mut offline_group = None;

        if online_count > 0 {
            let group = MemberListGroup {
                id: "online".to_string(),
                count: online_count,
            };
            groups.push(group.clone());
            online_group = Some(group);
        }

        if offline_count > 0 {
            let group = MemberListGroup {
                id: "offline".to_string(),
                count: offline_count,
            };
            groups.push(group.clone());
            offline_group = Some(group);
        }

        let online_span = if online_count > 0 {
            1 + online_count
        } else {
            0
        };
        let offline_span = if offline_count > 0 {
            1 + offline_count
        } else {
            0
        };
        let total_items = online_span + offline_span;

        let start = (range[0].max(0) as usize).min(total_items);
        let end = ((range[1] + 1).max(0) as usize).min(total_items);
        let mut sliced_items = Vec::with_capacity(end.saturating_sub(start));

        for idx in start..end {
            if online_span > 0 && idx < online_span {
                if idx == 0 {
                    if let Some(ref g) = online_group {
                        sliced_items.push(MemberListItem::Group { group: g.clone() });
                    }
                } else {
                    sliced_items.push(to_item(&self.online_members[idx - 1]));
                }
            } else {
                let off_idx = idx - online_span;
                if off_idx == 0 {
                    if let Some(ref g) = offline_group {
                        sliced_items.push(MemberListItem::Group { group: g.clone() });
                    }
                } else {
                    sliced_items.push(to_item(&self.offline_members[off_idx - 1]));
                }
            }
        }

        GuildMemberListUpdate {
            guild_id: guild_id.to_string(),
            channel_id: channel_id.to_string(),
            id: channel_id.to_string(),
            ops: vec![MemberListSyncOp {
                op: "SYNC",
                range,
                items: sliced_items,
            }],
            online_count,
            member_count: total_member_count,
            groups,
        }
    }
}

async fn fetch_snapshot(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
) -> Option<GuildMemberListSnapshot> {
    let channel_in_guild: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM channels WHERE id = ? AND guild_id = ? LIMIT 1")
            .bind(channel_id)
            .bind(guild_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    if channel_in_guild.is_none() {
        return None;
    }

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()?;

    let guild_roles: Vec<RolePerm> =
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

    let mut live_presences = state.hub.guild_live_presences(guild_id);

    let member_rows = match sqlx::query_as::<_, GuildMemberUserRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = ?
        ORDER BY COALESCE(gm.nick, u.global_name, u.username) ASC, gm.joined_at ASC
        "#,
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            error!(error = %err, guild_id = %guild_id, "Failed to fetch members for lazy request");
            return None;
        }
    };

    let role_rows = sqlx::query_as::<_, GuildMemberRole>(
        "SELECT user_id, role_id FROM guild_member_roles WHERE guild_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut roles_by_user: HashMap<String, Vec<String>> = HashMap::new();
    for r in role_rows {
        roles_by_user.entry(r.user_id).or_default().push(r.role_id);
    }

    let mut online_members = Vec::new();
    let mut offline_members = Vec::new();
    let mut viewer_allowed = HashSet::new();

    for row in member_rows {
        let roles = roles_by_user.remove(&row.user_id).unwrap_or_default();
        let base = permission_service::compute_base_permissions(
            &owner_id,
            guild_id,
            &row.user_id,
            &guild_roles,
            &roles,
        );
        let ch_perms = permission_service::compute_channel_permissions(
            base,
            guild_id,
            &row.user_id,
            &roles,
            &overwrites,
        );
        if ch_perms.contains(PermissionFlags::VIEW_CHANNEL | PermissionFlags::VIEW_CHANNEL_MEMBERS)
        {
            viewer_allowed.insert(row.user_id.clone());
        }
        if !ch_perms.contains(PermissionFlags::VIEW_CHANNEL) {
            continue;
        }

        let (status, afk, mobile, custom_status) = live_presences
            .remove(&row.user_id)
            .unwrap_or(("offline", false, false, None));

        let is_visibly_online = status != "offline";

        let nick = row.nick.clone();
        let joined_at = row.joined_at;
        let member = SnapshotMember {
            user: row.into_partial_user(),
            nick,
            roles,
            joined_at,
            status,
            mobile,
            afk,
            custom_status,
        };

        if is_visibly_online {
            online_members.push(member);
        } else {
            offline_members.push(member);
        }
    }

    Some(GuildMemberListSnapshot {
        online_members,
        offline_members,
        viewer_allowed,
    })
}

pub async fn handle(session_id: &str, state: &AppState, payload: LazyRequestPayload) {
    let actions = payload.into_actions();
    if actions.is_empty() {
        return;
    }

    let Some(session) = state.hub.get(session_id) else {
        return;
    };
    let Some(user_id) = session.user_id() else {
        return;
    };

    let mut to_send: Vec<(String, String, [i64; 2])> = Vec::new();
    let mut member_presences: Vec<(String, Vec<String>)> = Vec::new();

    for action in actions {
        match action {
            LazyListAction::Subscribe {
                guild_id,
                channel_id,
                range,
            } => {
                if !session.has_guild(&guild_id) {
                    continue;
                }
                let channel_in_guild = sqlx::query_scalar::<_, i32>(
                    "SELECT 1 FROM channels WHERE id = ? AND guild_id = ? LIMIT 1",
                )
                .bind(&channel_id)
                .bind(&guild_id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
                .is_some();
                if !channel_in_guild {
                    session.remove_member_list_subscription(&guild_id);
                    continue;
                }
                let start = range[0].max(0);
                let end = range[1]
                    .max(start)
                    .min(start.saturating_add(MEMBER_LIST_RANGE_MAX_SPAN));
                let range = [start, end];
                let allowed = permission_service::compute_user_channel_permissions(
                    state,
                    &guild_id,
                    &channel_id,
                    &user_id,
                )
                .await
                .is_ok_and(|p| {
                    p.contains(
                        PermissionFlags::VIEW_CHANNEL | PermissionFlags::VIEW_CHANNEL_MEMBERS,
                    )
                });
                if !allowed {
                    session.remove_member_list_subscription(&guild_id);
                    continue;
                }
                session.set_member_list_subscription(guild_id.clone(), channel_id.clone(), range);
                to_send.push((guild_id, channel_id, range));
            }
            LazyListAction::Unsubscribe { guild_id } => {
                session.remove_member_list_subscription(&guild_id);
            }
            LazyListAction::SubscribeMembers { guild_id, user_ids } => {
                if session.has_guild(&guild_id) {
                    member_presences.push((guild_id, user_ids));
                }
            }
        }
    }

    for (guild_id, user_ids) in member_presences {
        let mut seen = HashSet::new();
        let requested: Vec<String> = user_ids
            .into_iter()
            .filter(|id| seen.insert(id.clone()))
            .take(MAX_LAZY_SUBSCRIBED_MEMBERS)
            .collect();
        if requested.is_empty() {
            continue;
        }

        let mut qb = sqlx::QueryBuilder::<sqlx::MySql>::new(
            "SELECT user_id FROM guild_members WHERE guild_id = ",
        );
        qb.push_bind(&guild_id);
        qb.push(" AND user_id IN (");
        let mut sep = qb.separated(", ");
        for uid in &requested {
            sep.push_bind(uid);
        }
        qb.push(")");

        let valid_ids: Vec<String> = qb
            .build_query_as::<(String,)>()
            .fetch_all(&state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(uid,)| uid)
            .collect();

        let payloads = presence::build_presence_payloads(state, &valid_ids, Some(&guild_id)).await;
        for payload in payloads {
            session.dispatch(events::PRESENCE_UPDATE, payload);
        }
    }

    for (guild_id, channel_id, range) in to_send {
        let Some(snapshot) = fetch_snapshot(state, &guild_id, &channel_id).await else {
            continue;
        };
        let update = snapshot.build_update(&guild_id, &channel_id, range);
        session.dispatch(events::GUILD_MEMBER_LIST_UPDATE, update);
    }
}

pub async fn refresh_guild_member_lists(state: &AppState, guild_id: &str) {
    let subscribers = state.hub.guild_member_list_subscribers(guild_id);
    if subscribers.is_empty() {
        return;
    }

    let mut by_channel: HashMap<
        String,
        Vec<(
            std::sync::Arc<crate::gateway::session::GatewaySession>,
            [i64; 2],
        )>,
    > = HashMap::new();
    for (session, channel_id, range) in subscribers {
        by_channel
            .entry(channel_id)
            .or_default()
            .push((session, range));
    }

    for (channel_id, subs) in by_channel {
        let Some(snapshot) = fetch_snapshot(state, guild_id, &channel_id).await else {
            continue;
        };

        for (session, range) in subs {
            let Some(uid) = session.user_id() else {
                continue;
            };
            if !snapshot.viewer_allowed.contains(&uid) {
                session.remove_member_list_subscription(guild_id);
                continue;
            }
            let update = snapshot.build_update(guild_id, &channel_id, range);
            session.dispatch(events::GUILD_MEMBER_LIST_UPDATE, update);
        }
    }
}
