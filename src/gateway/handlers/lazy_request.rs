use std::collections::HashMap;

use serde_json::{Value, json};
use sqlx::FromRow;
use tracing::error;

use crate::AppState;
use crate::constants::gateway::events;
use crate::models::gateway::GatewayPayload;
use crate::models::guild_member::GuildMemberRole;
use crate::models::serde_helpers::millis_to_iso;

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
}

fn parse_subscription(d: &Value) -> Option<(String, String, [i64; 2])> {
    let mut range = [0i64, 99i64];

    if let Some(subs) = d.get("subscriptions").and_then(|s| s.as_object()) {
        let (guild_id, sub_val) = subs.iter().next()?;
        let mut channel_id = "everyone".to_string();

        if let Some(channels) = sub_val.get("member_list_channels").and_then(|c| c.as_object()) {
            if let Some((cid, ranges_val)) = channels.iter().next() {
                channel_id = cid.clone();
                if let Some(arr) = ranges_val
                    .as_array()
                    .and_then(|r| r.first())
                    .and_then(|r| r.as_array())
                {
                    if arr.len() == 2 {
                        range = [arr[0].as_i64().unwrap_or(0), arr[1].as_i64().unwrap_or(99)];
                    }
                }
            }
        }
        return Some((guild_id.clone(), channel_id, range));
    }

    let guild_id = d.get("guild_id").and_then(|g| g.as_str())?.to_string();
    let channel_id = d
        .get("channels")
        .and_then(|c| c.as_object())
        .and_then(|c| c.keys().next().cloned())
        .unwrap_or_else(|| "everyone".to_string());

    Some((guild_id, channel_id, range))
}

pub async fn handle(session_id: &str, state: &AppState, payload: &GatewayPayload) {
    let Some(d) = &payload.d else { return };
    let Some((target_guild_id, target_channel_id, range)) = parse_subscription(d) else {
        return;
    };

    // Only allow authenticated sessions belonging to the target guild.
    {
        let sessions = state.sessions.read().await;
        let Some(session) = sessions.get(session_id) else {
            return;
        };
        if session.user_id.is_none() || !session.guilds.contains(&target_guild_id) {
            return;
        }
    }

    let online_user_ids = state.online_user_ids().await;

    let member_rows = match sqlx::query_as::<_, LazyMemberRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
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
        "SELECT guild_id, user_id, role_id FROM guild_member_roles WHERE guild_id = ?",
    )
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
        let is_online = online_user_ids.contains(&row.user_id);
        let status = if is_online { "online" } else { "offline" };
        let roles = roles_by_user.remove(&row.user_id).unwrap_or_default();

        let item = json!({
            "member": {
                "user": {
                    "id": row.user_id,
                    "username": row.username,
                    "discriminator": row.discriminator,
                    "global_name": row.global_name,
                    "avatar": row.avatar,
                    "bot": row.bot,
                },
                "nick": row.nick,
                "roles": roles,
                "mute": false,
                "deaf": false,
                "joined_at": millis_to_iso(row.joined_at),
                "presence": {
                    "user": { "id": row.user_id },
                    "status": status,
                    "client_status": { "desktop": status },
                    "activities": [],
                }
            }
        });

        if is_online {
            online_members.push(item);
        } else {
            offline_members.push(item);
        }
    }

    let online_count = online_members.len();
    let offline_count = offline_members.len();
    let total_member_count = online_count + offline_count;

    let mut all_items = Vec::with_capacity(total_member_count + 2);
    if online_count > 0 {
        all_items.push(json!({ "group": { "id": "online", "count": online_count } }));
        all_items.extend(online_members);
    }
    if offline_count > 0 {
        all_items.push(json!({ "group": { "id": "offline", "count": offline_count } }));
        all_items.extend(offline_members);
    }

    let start = (range[0].max(0) as usize).min(all_items.len());
    let end = ((range[1] + 1).max(0) as usize).min(all_items.len());
    let sliced_items = if start < end {
        all_items[start..end].to_vec()
    } else {
        vec![]
    };

    let update = json!({
        "guild_id": target_guild_id,
        "channel_id": target_channel_id,
        "id": target_channel_id,
        "ops": [{
            "op": "SYNC",
            "range": range,
            "items": sliced_items,
        }],
        "online_count": online_count,
        "member_count": total_member_count,
        "groups": [
            { "id": "online", "count": online_count },
            { "id": "offline", "count": offline_count },
        ]
    });

    let sessions = state.sessions.read().await;
    if let Some(session) = sessions.get(session_id) {
        session.dispatch(events::GUILD_MEMBER_LIST_UPDATE, update).await;
    }
}