use std::collections::{HashMap, HashSet};

use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::PermissionFlags;
use crate::error::AppError;
use crate::gateway::handlers::presence;
use crate::models::gateway::events::{
    ForumUnreadEntry, ForumUnreadsEvent, RequestForumUnreadsPayload, ThreadListSyncEvent,
    ThreadMemberListItem, ThreadMemberListUpdateEvent,
};
use crate::models::gateway::presence::{GatewayPresence, PresenceUser};
use crate::models::guild_member::{GuildMemberResponse, GuildMemberUserRow};
use crate::models::thread::{ThreadDbRow, ThreadMember, ThreadMemberDbRow, ThreadResponse};
use crate::services::{permission_service, queries};
use crate::utils::snowflake::validate_snowflake;

use super::helpers::{check_thread_view_access, fetch_thread_row, hydrate_threads_batch};

pub async fn fetch_guild_ready_threads(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    is_bot: bool,
) -> Result<Vec<ThreadResponse>, AppError> {
    let mut conn = state.db.acquire().await?;
    let rows = if is_bot {
        let sql = format!(
            "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE ct.guild_id = ? AND ct.archived = FALSE ORDER BY CAST(c.id AS UNSIGNED) DESC",
            ThreadDbRow::SELECT_COLS
        );
        sqlx::query_as::<_, ThreadDbRow>(&sql)
            .bind(guild_id)
            .fetch_all(&mut *conn)
            .await?
    } else {
        let sql = format!(
            "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id INNER JOIN thread_members tm ON tm.thread_id = c.id WHERE ct.guild_id = ? AND tm.user_id = ? AND ct.archived = FALSE ORDER BY CAST(c.id AS UNSIGNED) DESC",
            ThreadDbRow::SELECT_COLS
        );
        sqlx::query_as::<_, ThreadDbRow>(&sql)
            .bind(guild_id)
            .bind(user_id)
            .fetch_all(&mut *conn)
            .await?
    };

    let mut visible_rows = Vec::with_capacity(rows.len());
    let mut perm_cache: HashMap<String, PermissionFlags> = HashMap::new();
    for r in rows {
        let perms = match perm_cache.get(&r.parent_id).copied() {
            Some(p) => p,
            None => {
                let (p, _) = permission_service::compute_user_thread_permissions_conn(
                    &mut conn,
                    guild_id,
                    &r.parent_id,
                    user_id,
                )
                .await
                .unwrap_or((PermissionFlags::empty(), PermissionFlags::empty()));
                perm_cache.insert(r.parent_id.clone(), p);
                p
            }
        };
        if !perms.contains(PermissionFlags::VIEW_CHANNEL) {
            continue;
        }
        if is_bot
            && r.kind == ChannelType::PrivateThread as u32
            && !perms.contains(PermissionFlags::MANAGE_THREADS)
        {
            // Keep if bot is explicitly a member
            let joined: Option<(i32,)> = sqlx::query_as(
                "SELECT 1 FROM thread_members WHERE thread_id = ? AND user_id = ? LIMIT 1",
            )
            .bind(&r.id)
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;
            if joined.is_none() {
                continue;
            }
        }
        visible_rows.push(r);
    }

    hydrate_threads_batch(&mut conn, visible_rows, user_id, false).await
}

pub async fn build_thread_list_sync(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<ThreadListSyncEvent, AppError> {
    let mut conn = state.db.acquire().await?;
    let sql = format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE ct.guild_id = ? AND ct.archived = FALSE ORDER BY CAST(c.id AS UNSIGNED) DESC",
        ThreadDbRow::SELECT_COLS
    );
    let rows = sqlx::query_as::<_, ThreadDbRow>(&sql)
        .bind(guild_id)
        .fetch_all(&mut *conn)
        .await?;

    let mut visible_rows = Vec::with_capacity(rows.len());
    let mut perm_cache: HashMap<String, PermissionFlags> = HashMap::new();
    let joined_rows: Vec<ThreadMemberDbRow> = sqlx::query_as(
        r#"
        SELECT tm.thread_id, tm.user_id, tm.join_timestamp, tm.flags, tm.muted, tm.mute_config
        FROM thread_members tm
        INNER JOIN channel_threads ct ON ct.channel_id = tm.thread_id
        WHERE ct.guild_id = ? AND ct.archived = FALSE AND tm.user_id = ?
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_all(&mut *conn)
    .await?;
    let joined_set: HashSet<String> = joined_rows.iter().map(|r| r.thread_id.clone()).collect();

    for r in rows {
        let perms = match perm_cache.get(&r.parent_id).copied() {
            Some(p) => p,
            None => {
                let (p, _) = permission_service::compute_user_thread_permissions_conn(
                    &mut conn,
                    guild_id,
                    &r.parent_id,
                    user_id,
                )
                .await
                .unwrap_or((PermissionFlags::empty(), PermissionFlags::empty()));
                perm_cache.insert(r.parent_id.clone(), p);
                p
            }
        };
        if !perms.contains(PermissionFlags::VIEW_CHANNEL) {
            continue;
        }
        if r.kind == ChannelType::PrivateThread as u32
            && !perms.contains(PermissionFlags::MANAGE_THREADS)
            && !joined_set.contains(&r.id)
        {
            continue;
        }
        visible_rows.push(r);
    }

    let visible_ids: HashSet<String> = visible_rows.iter().map(|r| r.id.clone()).collect();
    let threads = hydrate_threads_batch(&mut conn, visible_rows, user_id, false).await?;
    let members = joined_rows
        .into_iter()
        .filter(|m| visible_ids.contains(&m.thread_id))
        .map(|m| ThreadMember::from_db_self(m, true))
        .collect();

    Ok(ThreadListSyncEvent {
        guild_id: guild_id.to_string(),
        channel_ids: None,
        threads,
        members,
    })
}

pub async fn build_thread_member_list_update(
    state: &AppState,
    guild_id: &str,
    thread_id: &str,
    viewer_id: &str,
) -> Result<Option<ThreadMemberListUpdateEvent>, AppError> {
    if validate_snowflake("thread_id", thread_id).is_err() {
        return Ok(None);
    }
    let mut conn = state.db.acquire().await?;
    let Some(row) = fetch_thread_row(&mut *conn, thread_id).await? else {
        return Ok(None);
    };
    if row.guild_id != guild_id {
        return Ok(None);
    }
    if check_thread_view_access(&mut conn, &row, viewer_id)
        .await
        .is_err()
    {
        return Ok(None);
    }

    let member_rows: Vec<ThreadMemberDbRow> = sqlx::query_as(
        "SELECT thread_id, user_id, join_timestamp, flags, muted, mute_config FROM thread_members WHERE thread_id = ? ORDER BY join_timestamp ASC LIMIT 1000",
    )
    .bind(thread_id)
    .fetch_all(&mut *conn)
    .await?;
    drop(conn);

    if member_rows.is_empty() {
        return Ok(Some(ThreadMemberListUpdateEvent {
            guild_id: guild_id.to_string(),
            thread_id: thread_id.to_string(),
            members: Vec::new(),
        }));
    }

    let user_ids: Vec<String> = member_rows.iter().map(|m| m.user_id.clone()).collect();
    let mut mqb = QueryBuilder::<MySql>::new(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = 
        "#,
    );
    mqb.push_bind(guild_id);
    mqb.push(" AND gm.user_id IN (");
    let mut sep = mqb.separated(", ");
    for uid in &user_ids {
        sep.push_bind(uid);
    }
    mqb.push(")");

    let gm_rows = mqb
        .build_query_as::<GuildMemberUserRow>()
        .fetch_all(&state.db)
        .await?;
    let mut roles_by_user = queries::fetch_guild_member_roles_map(&state.db, guild_id).await?;
    let mut gm_by_user: HashMap<String, GuildMemberResponse> = gm_rows
        .into_iter()
        .map(|mr| {
            let roles = roles_by_user.remove(&mr.user_id).unwrap_or_default();
            let uid = mr.user_id.clone();
            (uid, mr.into_member_response(roles))
        })
        .collect();

    let presences_list = presence::build_presence_payloads(state, &user_ids, Some(guild_id)).await;
    let mut presence_by_user: HashMap<String, GatewayPresence> = presences_list
        .into_iter()
        .map(|p| {
            let uid = match &p.user {
                PresenceUser::Partial(u) => u.id.clone(),
                PresenceUser::IdOnly { id } => id.clone(),
            };
            (uid, p)
        })
        .collect();

    let items = member_rows
        .into_iter()
        .map(|m| {
            let gm = gm_by_user.remove(&m.user_id);
            let pres = presence_by_user.remove(&m.user_id).unwrap_or(GatewayPresence {
                user: match &gm {
                    Some(g) => PresenceUser::Partial(g.user.clone()),
                    None => PresenceUser::IdOnly {
                        id: m.user_id.clone(),
                    },
                },
                status: "offline".to_string(),
                mobile: false,
                afk: false,
                custom_status: None,
                guild_id: Some(guild_id.to_string()),
            });
            ThreadMemberListItem {
                user_id: m.user_id,
                join_timestamp: m.join_timestamp,
                flags: m.flags,
                member: gm,
                presence: pres,
            }
        })
        .collect();

    Ok(Some(ThreadMemberListUpdateEvent {
        guild_id: guild_id.to_string(),
        thread_id: thread_id.to_string(),
        members: items,
    }))
}

pub async fn handle_request_forum_unreads(
    state: &AppState,
    user_id: &str,
    payload: RequestForumUnreadsPayload,
) -> Option<ForumUnreadsEvent> {
    if validate_snowflake("guild_id", &payload.guild_id).is_err()
        || validate_snowflake("channel_id", &payload.channel_id).is_err()
    {
        return None;
    }

    let channel_row: Option<(String, u32)> = sqlx::query_as(
        "SELECT guild_id, type FROM channels WHERE id = ? AND guild_id = ? LIMIT 1",
    )
    .bind(&payload.channel_id)
    .bind(&payload.guild_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let (_, kind) = channel_row?;
    if !ChannelType::is_forum_like(kind) {
        return None;
    }

    if permission_service::require_channel_permission(
        state,
        &payload.guild_id,
        &payload.channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await
    .is_err()
    {
        return None;
    }

    let requested: Vec<_> = payload.threads.into_iter().take(40).collect();
    if requested.is_empty() {
        return Some(ForumUnreadsEvent {
            guild_id: payload.guild_id,
            channel_id: payload.channel_id,
            threads: Vec::new(),
        });
    }

    let valid_req_ids: Vec<String> = requested
        .iter()
        .filter(|item| validate_snowflake("thread_id", &item.thread_id).is_ok())
        .map(|item| item.thread_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if valid_req_ids.is_empty() {
        return Some(ForumUnreadsEvent {
            guild_id: payload.guild_id,
            channel_id: payload.channel_id,
            threads: Vec::new(),
        });
    }

    let mut qb =
        QueryBuilder::<MySql>::new("SELECT channel_id FROM channel_threads WHERE parent_id = ");
    qb.push_bind(&payload.channel_id);
    qb.push(" AND channel_id IN (");
    let mut sep = qb.separated(", ");
    for id in &valid_req_ids {
        sep.push_bind(id);
    }
    qb.push(")");
    let found_ids: HashSet<String> = qb
        .build_query_scalar::<String>()
        .fetch_all(&state.db)
        .await
        .ok()?
        .into_iter()
        .collect();

    let mut entries = Vec::new();
    for item in requested {
        if !found_ids.contains(&item.thread_id) {
            continue;
        }
        let ack_num = item
            .ack_message_id
            .as_deref()
            .filter(|ack| validate_snowflake("ack_message_id", ack).is_ok())
            .and_then(|ack| ack.parse::<u64>().ok());

        match ack_num {
            Some(ack) => {
                let count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM messages WHERE channel_id = ? AND id_num > ? LIMIT 26",
                )
                .bind(&item.thread_id)
                .bind(ack)
                .fetch_one(&state.db)
                .await
                .unwrap_or(0);
                entries.push(ForumUnreadEntry {
                    thread_id: item.thread_id,
                    count: Some((count.min(25)) as u32),
                    missing: None,
                });
            }
            None => {
                entries.push(ForumUnreadEntry {
                    thread_id: item.thread_id,
                    count: None,
                    missing: Some(true),
                });
            }
        }
    }

    Some(ForumUnreadsEvent {
        guild_id: payload.guild_id,
        channel_id: payload.channel_id,
        threads: entries,
    })
}
