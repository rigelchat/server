use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

use sqlx::FromRow;
use tracing::error;

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::{MEMBER_LIST_RANGE_MAX_SPAN, events};
use crate::constants::limits::MAX_LAZY_SUBSCRIBED_MEMBERS;
use crate::models::gateway::lazy_request::{
    GuildMemberListUpdate, LazyListAction, LazyRequestPayload, MemberListGroup, MemberListItem,
    MemberListMember, MemberListPresence, MemberListSyncOp,
};
use crate::models::guild_member::{GuildMemberFields, GuildMemberUserRow};
use crate::models::role::RolePerm;
use crate::models::user::{CustomStatus, PartialUser};
use crate::services::{permission_service, queries};
use crate::utils::snowflake::Snowflake;

use super::presence;

#[derive(FromRow)]
struct SnapshotRoleRow {
    id: String,
    position: u32,
    hoist: bool,
    hoist_position: Option<i32>,
    permissions: u64,
}

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

struct MemberListBucket {
    id: String,
    members: Vec<SnapshotMember>,
}

struct GuildMemberListSnapshot {
    buckets: Vec<MemberListBucket>,
    online_count: usize,
    member_count: usize,
    viewer_allowed: HashSet<String>,
}

fn compare_hoisted_roles(a: &SnapshotRoleRow, b: &SnapshotRoleRow) -> Ordering {
    let rank_a = a.hoist_position.unwrap_or(a.position as i32);
    let rank_b = b.hoist_position.unwrap_or(b.position as i32);
    rank_b
        .cmp(&rank_a)
        .then_with(|| b.position.cmp(&a.position))
        .then_with(|| Snowflake::compare_strings(&b.id, &a.id))
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
                member: Box::new(MemberListMember {
                    user: m.user.clone(),
                    member: GuildMemberFields::new(m.nick.clone(), m.roles.clone(), m.joined_at),
                    presence,
                }),
            }
        };

        let groups: Vec<MemberListGroup> = self
            .buckets
            .iter()
            .map(|b| MemberListGroup {
                id: b.id.clone(),
                count: b.members.len(),
            })
            .collect();

        let total_items: usize = self.buckets.iter().map(|b| 1 + b.members.len()).sum();
        let start = (range[0].max(0) as usize).min(total_items);
        let end = ((range[1] + 1).max(0) as usize).min(total_items);
        let mut sliced_items = Vec::with_capacity(end.saturating_sub(start));

        let mut cursor = 0usize;
        for bucket in &self.buckets {
            let bucket_len = 1 + bucket.members.len();
            let bucket_end = cursor + bucket_len;
            if bucket_end <= start {
                cursor = bucket_end;
                continue;
            }
            if cursor >= end {
                break;
            }

            let local_start = start.saturating_sub(cursor);
            let local_end = (end - cursor).min(bucket_len);
            for local_idx in local_start..local_end {
                if local_idx == 0 {
                    sliced_items.push(MemberListItem::Group {
                        group: MemberListGroup {
                            id: bucket.id.clone(),
                            count: bucket.members.len(),
                        },
                    });
                } else {
                    sliced_items.push(to_item(&bucket.members[local_idx - 1]));
                }
            }
            cursor = bucket_end;
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
            online_count: self.online_count,
            member_count: self.member_count,
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
    channel_in_guild?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()?;

    let role_rows: Vec<SnapshotRoleRow> = match sqlx::query_as(
        "SELECT id, position, hoist, hoist_position, permissions FROM roles WHERE guild_id = ?",
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            error!(error = %err, guild_id = %guild_id, "Failed to fetch roles for lazy request");
            return None;
        }
    };

    let guild_roles: Vec<RolePerm> = role_rows
        .iter()
        .map(|r| RolePerm {
            id: r.id.clone(),
            permissions: r.permissions,
        })
        .collect();

    let mut hoisted_roles: Vec<SnapshotRoleRow> = role_rows
        .into_iter()
        .filter(|r| r.hoist && r.id != guild_id)
        .collect();
    hoisted_roles.sort_by(compare_hoisted_roles);

    let hoisted_rank_by_id: HashMap<String, usize> = hoisted_roles
        .iter()
        .enumerate()
        .map(|(idx, r)| (r.id.clone(), idx))
        .collect();
    let mut hoisted_buckets: Vec<Vec<SnapshotMember>> =
        (0..hoisted_roles.len()).map(|_| Vec::new()).collect();

    let overwrites = match queries::fetch_channel_overwrites(&state.db, channel_id).await {
        Ok(ows) => ows,
        Err(err) => {
            error!(
                error = %err,
                channel_id = %channel_id,
                "Failed to fetch overwrites for lazy request"
            );
            return None;
        }
    };

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

    let mut roles_by_user = match queries::fetch_guild_member_roles_map(&state.db, guild_id).await {
        Ok(map) => map,
        Err(err) => {
            error!(
                error = %err,
                guild_id = %guild_id,
                "Failed to fetch member roles for lazy request"
            );
            return None;
        }
    };

    let mut online_unhoisted = Vec::new();
    let mut offline_members = Vec::new();
    let mut viewer_allowed = HashSet::new();
    let mut online_count = 0usize;

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
        let hoisted_idx = if is_visibly_online {
            roles
                .iter()
                .filter_map(|rid| hoisted_rank_by_id.get(rid).copied())
                .min()
        } else {
            None
        };

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
            online_count += 1;
            if let Some(idx) = hoisted_idx {
                hoisted_buckets[idx].push(member);
            } else {
                online_unhoisted.push(member);
            }
        } else {
            offline_members.push(member);
        }
    }

    let member_count = online_count + offline_members.len();
    let mut buckets = Vec::with_capacity(hoisted_roles.len() + 2);
    for (role, members) in hoisted_roles.into_iter().zip(hoisted_buckets) {
        if !members.is_empty() {
            buckets.push(MemberListBucket {
                id: role.id,
                members,
            });
        }
    }
    if !online_unhoisted.is_empty() {
        buckets.push(MemberListBucket {
            id: "online".to_string(),
            members: online_unhoisted,
        });
    }
    if !offline_members.is_empty() && offline_members.len() <= 1000 {
        buckets.push(MemberListBucket {
            id: "offline".to_string(),
            members: offline_members,
        });
    }

    Some(GuildMemberListSnapshot {
        buckets,
        online_count,
        member_count,
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

    let can_use_threads = session.has_channel_threads();
    let mut to_send: Vec<(String, String, [i64; 2])> = Vec::new();
    let mut member_presences: Vec<(String, Vec<String>)> = Vec::new();
    let mut thread_sync_guilds: Vec<String> = Vec::new();
    let mut thread_member_list_reqs: Vec<(String, Vec<String>)> = Vec::new();

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
            LazyListAction::SetGuildActive { guild_id, active } => {
                if session.has_guild(&guild_id) {
                    session.set_guild_active(&guild_id, active);
                }
            }
            LazyListAction::SubscribeThreads { guild_id, enabled } => {
                if can_use_threads && session.has_guild(&guild_id) {
                    session.set_thread_subscription(&guild_id, enabled);
                    if enabled {
                        thread_sync_guilds.push(guild_id);
                    }
                }
            }
            LazyListAction::SubscribeThreadMemberLists {
                guild_id,
                thread_ids,
            } => {
                if can_use_threads && session.has_guild(&guild_id) {
                    let mut seen = HashSet::new();
                    let capped: Vec<String> = thread_ids
                        .into_iter()
                        .filter(|id| seen.insert(id.clone()))
                        .take(10)
                        .collect();
                    let newly_added = session.set_thread_member_lists(&guild_id, capped);
                    if !newly_added.is_empty() {
                        thread_member_list_reqs.push((guild_id, newly_added));
                    }
                }
            }
        }
    }

    for guild_id in thread_sync_guilds {
        if let Ok(sync_evt) =
            crate::services::thread::build_thread_list_sync(state, &guild_id, &user_id).await
        {
            session.dispatch(events::THREAD_LIST_SYNC, sync_evt);
        }
    }

    for (guild_id, thread_ids) in thread_member_list_reqs {
        for thread_id in thread_ids {
            if let Ok(Some(evt)) = crate::services::thread::build_thread_member_list_update(
                state,
                &guild_id,
                &thread_id,
                &user_id,
            )
            .await
            {
                session.dispatch(events::THREAD_MEMBER_LIST_UPDATE, evt);
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

pub async fn refresh_thread_member_lists(state: &AppState, guild_id: &str, thread_id: &str) {
    let subs = state.hub.thread_member_list_subscribers(guild_id, thread_id);
    for session in subs {
        let Some(uid) = session.user_id() else {
            continue;
        };
        if let Ok(Some(evt)) =
            crate::services::thread::build_thread_member_list_update(state, guild_id, thread_id, &uid)
                .await
        {
            session.dispatch(events::THREAD_MEMBER_LIST_UPDATE, evt);
        }
    }
}

type ChannelSubscribers = Vec<(
    std::sync::Arc<crate::gateway::session::GatewaySession>,
    [i64; 2],
)>;

pub async fn refresh_guild_member_lists(state: &AppState, guild_id: &str) {
    let subscribers = state.hub.guild_member_list_subscribers(guild_id);
    if subscribers.is_empty() {
        return;
    }

    let mut by_channel: HashMap<String, ChannelSubscribers> = HashMap::new();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn test_member(id: &str, status: &'static str) -> SnapshotMember {
        SnapshotMember {
            user: PartialUser {
                id: id.to_string(),
                username: format!("user_{id}"),
                discriminator: "0000".to_string(),
                global_name: None,
                avatar: None,
                avatar_color: None,
                bot: false,
                flags: 0,
            },
            nick: None,
            roles: vec![],
            joined_at: 0,
            status,
            mobile: false,
            afk: false,
            custom_status: None,
        }
    }

    #[test]
    fn hoisted_groups_and_range_slicing_work() {
        let snapshot = GuildMemberListSnapshot {
            buckets: vec![
                MemberListBucket {
                    id: "role_admin".to_string(),
                    members: vec![test_member("u1", "online")],
                },
                MemberListBucket {
                    id: "online".to_string(),
                    members: vec![test_member("u2", "online")],
                },
                MemberListBucket {
                    id: "offline".to_string(),
                    members: vec![test_member("u3", "offline")],
                },
            ],
            online_count: 2,
            member_count: 3,
            viewer_allowed: HashSet::new(),
        };

        let full = snapshot.build_update("g1", "c1", [0, 99]);
        assert_eq!(full.groups.len(), 3);
        assert_eq!(full.online_count, 2);
        assert_eq!(full.member_count, 3);
        assert_eq!(full.ops[0].items.len(), 6);

        let sliced = snapshot.build_update("g1", "c1", [1, 3]);
        assert_eq!(sliced.ops[0].items.len(), 3);
        assert!(matches!(
            sliced.ops[0].items[0],
            MemberListItem::Member { .. }
        ));
        assert!(matches!(
            sliced.ops[0].items[1],
            MemberListItem::Group { .. }
        ));
        assert!(matches!(
            sliced.ops[0].items[2],
            MemberListItem::Member { .. }
        ));
    }

    #[test]
    fn hoisted_roles_sort_descending_by_hoist_position_with_position_fallback() {
        let roi = SnapshotRoleRow {
            id: "1".into(),
            position: 39,
            hoist: true,
            hoist_position: Some(21),
            permissions: 0,
        };
        let dev = SnapshotRoleRow {
            id: "2".into(),
            position: 37,
            hoist: true,
            hoist_position: Some(19),
            permissions: 0,
        };
        let hokage = SnapshotRoleRow {
            id: "3".into(),
            position: 30,
            hoist: true,
            hoist_position: Some(20),
            permissions: 0,
        };
        let membre = SnapshotRoleRow {
            id: "4".into(),
            position: 19,
            hoist: true,
            hoist_position: Some(1),
            permissions: 0,
        };
        let new_top = SnapshotRoleRow {
            id: "5".into(),
            position: 40,
            hoist: true,
            hoist_position: None,
            permissions: 0,
        };

        let mut list = [membre, dev, hokage, roi, new_top];
        list.sort_by(compare_hoisted_roles);
        let ids: Vec<&str> = list.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["5", "1", "3", "2", "4"]);
    }
}
