use std::collections::HashMap;

use sqlx::FromRow;
use tracing::error;

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::events;
use crate::models::gateway::lazy_request::{
    GuildMemberListUpdate, LazyRequestPayload, MemberListClientStatus, MemberListGroup,
    MemberListItem, MemberListMember, MemberListPresence, MemberListSyncOp,
};
use crate::models::guild_member::{GuildMemberRole, GuildMemberUserRef};
use crate::models::user::{PartialUser, UserSettingsExtras};

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
    status: String,
    extra_settings: Option<String>,
}

pub async fn handle(session_id: &str, state: &AppState, payload: LazyRequestPayload) {
    let Some((target_guild_id, target_channel_id, range)) = payload.extract() else {
        return;
    };

    let dialect = {
        let sessions = state.sessions.read().await;
        let Some(session) = sessions.get(session_id) else {
            return;
        };
        if session.user_id.is_none() || !session.guilds.contains(&target_guild_id) {
            return;
        }
        session.dialect
    };

    let online_user_ids = state.online_user_ids().await;

    let member_rows = match sqlx::query_as::<_, LazyMemberRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags,
            COALESCE(us.status, 'online') AS status, us.extra_settings
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        LEFT JOIN user_settings us ON us.id = u.id
        WHERE gm.guild_id = ?
        ORDER BY gm.joined_at ASC
        "#,
    )
    .bind(&target_guild_id)
    .fetch_all(&state.db)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            error!(error = %err, guild_id = %target_guild_id, "Failed to fetch members for lazy request");
            return;
        }
    };

    let role_rows = sqlx::query_as::<_, GuildMemberRole>(
        "SELECT user_id, role_id FROM guild_member_roles WHERE guild_id = ? AND role_id != ?",
    )
    .bind(&target_guild_id)
    .bind(&target_guild_id)
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
        let is_visibly_online =
            online_user_ids.contains(&row.user_id) && row.status != "invisible";

        let status = if is_visibly_online {
            row.status.clone()
        } else {
            "offline".to_string()
        };

        let custom_status = if is_visibly_online {
            row.extra_settings
                .as_deref()
                .and_then(|s| serde_json::from_str::<UserSettingsExtras>(s).ok())
                .and_then(|e| e.custom_status)
        } else {
            None
        };

        let roles = roles_by_user.remove(&row.user_id).unwrap_or_default();
        let presence = match dialect {
            ClientDialect::Fluxer => MemberListPresence::Fluxer {
                status,
                mobile: false,
                afk: false,
                custom_status,
            },
            ClientDialect::Discord => {
                let desktop = (status != "offline").then(|| status.clone());
                MemberListPresence::Discord {
                    user: GuildMemberUserRef {
                        id: row.user_id.clone(),
                    },
                    status,
                    client_status: MemberListClientStatus { desktop },
                    activities: vec![],
                }
            }
        };

        let item = MemberListItem::Member {
            member: MemberListMember {
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
                avatar: None,
                banner: None,
                accent_color: None,
                roles,
                joined_at: row.joined_at,
                mute: false,
                deaf: false,
                communication_disabled_until: None,
                presence,
            },
        };

        if is_visibly_online {
            online_members.push(item);
        } else {
            offline_members.push(item);
        }
    }

    let online_count = online_members.len();
    let offline_count = offline_members.len();
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
        all_items.extend(online_members);
    }

    if offline_count > 0 {
        let group = MemberListGroup {
            id: "offline".to_string(),
            count: offline_count,
        };
        groups.push(group.clone());
        all_items.push(MemberListItem::Group { group });
        all_items.extend(offline_members);
    }

    let start = (range[0].max(0) as usize).min(all_items.len());
    let end = ((range[1] + 1).max(0) as usize).min(all_items.len());
    let sliced_items = if start < end {
        all_items[start..end].to_vec()
    } else {
        vec![]
    };

    let update = GuildMemberListUpdate {
        guild_id: target_guild_id,
        channel_id: target_channel_id.clone(),
        id: target_channel_id,
        ops: vec![MemberListSyncOp {
            op: "SYNC",
            range,
            items: sliced_items,
        }],
        online_count,
        member_count: total_member_count,
        groups,
    };

    let sessions = state.sessions.read().await;
    if let Some(session) = sessions.get(session_id) {
        session.dispatch(events::GUILD_MEMBER_LIST_UPDATE, update).await;
    }
}