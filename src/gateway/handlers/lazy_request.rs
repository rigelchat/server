use std::collections::HashMap;

use sqlx::FromRow;
use tracing::error;

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::events;
use crate::models::gateway::lazy_request::{
    GuildMemberListUpdate, LazyListAction, LazyRequestPayload, MemberListClientStatus,
    MemberListGroup, MemberListItem, MemberListMember, MemberListPresence, MemberListSyncOp,
};
use crate::models::guild_member::{GuildMemberRole, GuildMemberUserRef};
use crate::models::user::{CustomStatus, PartialUser, UserSettingsExtras};

use super::presence;

#[derive(FromRow)]
struct LazyMemberRow {
    user_id: String,
    joined_at: u64,
    nick: Option<String>,
    username: String,
    discriminator: String,
    global_name: Option<String>,
    avatar: Option<String>,
    bot: bool,
    public_flags: u32,
    extra_settings: Option<String>,
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

struct GuildMemberListSnapshot {
    online_members: Vec<SnapshotMember>,
    offline_members: Vec<SnapshotMember>,
}

impl GuildMemberListSnapshot {
    fn build_update(
        &self,
        guild_id: &str,
        channel_id: &str,
        range: [i64; 2],
        dialect: ClientDialect,
    ) -> GuildMemberListUpdate {
        let to_item = |m: &SnapshotMember| -> MemberListItem {
            let presence = match dialect {
                ClientDialect::Fluxer => MemberListPresence::Fluxer {
                    status: m.status.to_string(),
                    mobile: m.mobile,
                    afk: m.afk,
                    custom_status: m.custom_status.clone(),
                },
                ClientDialect::Discord => {
                    let desktop = (m.status != "offline").then(|| m.status.to_string());
                    MemberListPresence::Discord {
                        user: GuildMemberUserRef {
                            id: m.user.id.clone(),
                        },
                        status: m.status.to_string(),
                        client_status: MemberListClientStatus { desktop },
                        activities: vec![],
                    }
                }
            };

            MemberListItem::Member {
                member: MemberListMember {
                    user: m.user.clone(),
                    nick: m.nick.clone(),
                    avatar: None,
                    banner: None,
                    accent_color: None,
                    roles: m.roles.clone(),
                    joined_at: m.joined_at,
                    mute: false,
                    deaf: false,
                    communication_disabled_until: None,
                    presence,
                },
            }
        };

        let online_count = self.online_members.len();
        let offline_count = self.offline_members.len();
        let total_member_count = online_count + offline_count;

        let mut groups = Vec::new();
        let mut all_items = Vec::with_capacity(total_member_count + 2);

        if online_count > 0 {
            let group = MemberListGroup {
                id: "online".to_string(),
                count: online_count,
            };
            groups.push(group.clone());
            all_items.push(MemberListItem::Group { group });
            all_items.extend(self.online_members.iter().map(to_item));
        }

        if offline_count > 0 {
            let group = MemberListGroup {
                id: "offline".to_string(),
                count: offline_count,
            };
            groups.push(group.clone());
            all_items.push(MemberListItem::Group { group });
            all_items.extend(self.offline_members.iter().map(to_item));
        }

        let start = (range[0].max(0) as usize).min(all_items.len());
        let end = ((range[1] + 1).max(0) as usize).min(all_items.len());
        let sliced_items = if start < end {
            all_items[start..end].to_vec()
        } else {
            vec![]
        };

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

async fn fetch_snapshot(state: &AppState, guild_id: &str) -> Option<GuildMemberListSnapshot> {
    let live_states: HashMap<String, (Vec<String>, bool, bool)> = {
        let sessions = state.sessions.read().await;
        let mut map: HashMap<String, (Vec<String>, bool, bool)> = HashMap::new();
        for s in sessions.values() {
            let Some(uid) = &s.user_id else {
                continue;
            };
            let entry = map.entry(uid.clone()).or_insert_with(|| (Vec::new(), true, false));
            entry.0.push(s.status.clone());
            entry.1 &= s.afk;
            entry.2 |= s.mobile;
        }
        map
    };

    let member_rows = match sqlx::query_as::<_, LazyMemberRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags,
            us.extra_settings
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        LEFT JOIN user_settings us ON us.id = u.id
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

    for row in member_rows {
        let (status, afk, mobile) = match live_states.get(&row.user_id) {
            Some((statuses, all_afk, any_mobile)) => {
                let combined = presence::combined_status(statuses.iter().map(String::as_str));
                let pub_st = presence::public_status(combined);
                (pub_st, *all_afk && !any_mobile, *any_mobile)
            }
            None => ("offline", false, false),
        };

        let is_visibly_online = status != "offline";
        let custom_status = if is_visibly_online {
            row.extra_settings
                .as_deref()
                .and_then(|s| serde_json::from_str::<UserSettingsExtras>(s).ok())
                .and_then(|e| e.custom_status)
        } else {
            None
        };

        let roles = roles_by_user.remove(&row.user_id).unwrap_or_default();
        let member = SnapshotMember {
            user: PartialUser {
                id: row.user_id,
                username: row.username,
                discriminator: row.discriminator,
                global_name: row.global_name,
                avatar: row.avatar,
                avatar_color: None,
                bot: row.bot,
                flags: row.public_flags,
            },
            nick: row.nick,
            roles,
            joined_at: row.joined_at,
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
    })
}

pub async fn handle(session_id: &str, state: &AppState, payload: LazyRequestPayload) {
    let actions = payload.into_actions();
    if actions.is_empty() {
        return;
    }

    let mut to_send: Vec<(String, String, [i64; 2], ClientDialect)> = Vec::new();
    let mut member_presences: Vec<(String, Vec<String>, ClientDialect)> = Vec::new();

    {
        let mut sessions = state.sessions.write().await;
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        if session.user_id.is_none() {
            return;
        }

        for action in actions {
            match action {
                LazyListAction::Subscribe {
                    guild_id,
                    channel_id,
                    range,
                } => {
                    if !session.guilds.contains(&guild_id) {
                        continue;
                    }
                    session
                        .member_list_subscriptions
                        .insert(guild_id.clone(), (channel_id.clone(), range));
                    to_send.push((guild_id, channel_id, range, session.dialect));
                }
                LazyListAction::Unsubscribe { guild_id } => {
                    session.member_list_subscriptions.remove(&guild_id);
                }
                LazyListAction::SubscribeMembers { guild_id, user_ids } => {
                    if session.guilds.contains(&guild_id) {
                        member_presences.push((guild_id, user_ids, session.dialect));
                    }
                }
            }
        }
    }

    for (guild_id, user_ids, dialect) in member_presences {
        for uid in user_ids.into_iter().take(1000) {
            let payload =
                presence::build_presence_payload(state, &uid, Some(&guild_id), dialect).await;
            let sessions = state.sessions.read().await;
            if let Some(session) = sessions.get(session_id) {
                session.dispatch(events::PRESENCE_UPDATE, payload).await;
            }
        }
    }

    for (guild_id, channel_id, range, dialect) in to_send {
        let Some(snapshot) = fetch_snapshot(state, &guild_id).await else {
            continue;
        };
        let update = snapshot.build_update(&guild_id, &channel_id, range, dialect);
        let sessions = state.sessions.read().await;
        if let Some(session) = sessions.get(session_id) {
            session
                .dispatch(events::GUILD_MEMBER_LIST_UPDATE, update)
                .await;
        }
    }
}

pub async fn refresh_guild_member_lists(state: &AppState, guild_id: &str) {
    let subscribers: Vec<(String, ClientDialect, String, [i64; 2])> = {
        let sessions = state.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.user_id.is_some() && s.guilds.contains(guild_id))
            .filter_map(|s| {
                s.member_list_subscriptions
                    .get(guild_id)
                    .map(|(cid, range)| (s.id.clone(), s.dialect, cid.clone(), *range))
            })
            .collect()
    };

    if subscribers.is_empty() {
        return;
    }

    let Some(snapshot) = fetch_snapshot(state, guild_id).await else {
        return;
    };

    let sessions = state.sessions.read().await;
    for (sid, dialect, channel_id, range) in subscribers {
        if let Some(session) = sessions.get(&sid) {
            let update = snapshot.build_update(guild_id, &channel_id, range, dialect);
            session
                .dispatch(events::GUILD_MEMBER_LIST_UPDATE, update)
                .await;
        }
    }
}