use std::collections::HashMap;

use chrono::Utc;
use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::{PermissionFlags, ThreadMemberFlags};
use crate::constants::gateway::events;
use crate::constants::limits::{
    DEFAULT_THREAD_MEMBERS_LIMIT, MAX_THREAD_MEMBERS, MAX_THREAD_MEMBERS_LIMIT,
};
use crate::constants::message::MessageType;
use crate::error::AppError;
use crate::gateway::handlers::presence;
use crate::models::gateway::events::{
    GatewayMessageEvent, ThreadCreateEvent, ThreadMemberUpdateEvent, ThreadMembersUpdateEvent,
};
use crate::models::guild_member::{GuildMemberFields, GuildMemberResponse, GuildMemberUserRow};
use crate::models::message::Message;
use crate::models::rest::threads::{ListThreadMembersQuery, ModifyThreadMemberSettingsRequest};
use crate::models::thread::{ThreadMember, ThreadMemberRow};
use crate::services::{member_service, permission_service, queries, user};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake, validate_snowflake_opt};

use super::helpers::{
    add_thread_member_tx, check_thread_view_access, fetch_thread_member_self, fetch_thread_row,
    hydrate_thread, remove_thread_member_tx,
};

async fn ensure_thread_row(
    conn: &mut sqlx::MySqlConnection,
    thread_id: &str,
) -> Result<crate::models::thread::ThreadRow, AppError> {
    if let Some(row) = fetch_thread_row(&mut *conn, thread_id).await? {
        return Ok(row);
    }
    let ch_exists: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM channels WHERE id = ? LIMIT 1")
        .bind(thread_id)
        .fetch_optional(&mut *conn)
        .await?;
    if ch_exists.is_some() {
        Err(AppError::invalid_channel_type())
    } else {
        Err(AppError::unknown_channel())
    }
}

async fn fetch_guild_member_response(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Option<GuildMemberResponse> {
    let row = sqlx::query_as::<_, GuildMemberUserRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = ? AND gm.user_id = ?
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()?;
    let roles = queries::fetch_member_roles(&state.db, guild_id, user_id)
        .await
        .unwrap_or_default();
    Some(row.into_member_response(roles))
}

pub async fn join_thread(
    state: &AppState,
    thread_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<(), AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }

    let mut tx = state.db.begin().await?;
    let row = ensure_thread_row(&mut tx, thread_id).await?;
    let (parent_perms, _, already_member) =
        check_thread_view_access(&mut tx, &row, user_id).await?;
    let is_mod = parent_perms.contains(PermissionFlags::MANAGE_THREADS);

    if row.archived {
        return Err(AppError::thread_archived());
    }
    if row.locked && !is_mod {
        return Err(AppError::thread_locked());
    }
    if already_member {
        tx.commit().await?;
        return Ok(());
    }

    let current_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_members WHERE thread_id = ? FOR UPDATE")
            .bind(thread_id)
            .fetch_one(&mut *tx)
            .await?;
    if current_count >= MAX_THREAD_MEMBERS {
        return Err(AppError::max_thread_members());
    }

    let now = Utc::now().timestamp_millis() as u64;
    let (_, inserted_row, member_count) =
        add_thread_member_tx(&mut tx, thread_id, user_id, 0, now).await?;
    let hydrated = hydrate_thread(&mut tx, row.clone(), user_id, false).await?;
    tx.commit().await?;

    let gm = fetch_guild_member_response(state, &row.guild_id, user_id).await;
    let pres = presence::build_presence_payloads(state, &[user_id.to_string()], None)
        .await
        .into_iter()
        .find(|p| p.status != "offline");
    let mut added = ThreadMember::from_db_public(inserted_row, None);
    added.member = Some(gm);
    added.presence = Some(pres);

    permission_service::dispatch_channel_with_ctx(
        state,
        &row.guild_id,
        &row.parent_id,
        None,
        events::THREAD_MEMBERS_UPDATE,
        &ThreadMembersUpdateEvent {
            id: thread_id.to_string(),
            guild_id: row.guild_id.clone(),
            member_count,
            added_members: Some(vec![added]),
            removed_member_ids: None,
        },
    )
    .await;

    state
        .dispatch_user(
            events::THREAD_CREATE,
            &ThreadCreateEvent {
                thread: hydrated,
                newly_created: None,
            },
            user_id,
        )
        .await;

    crate::gateway::handlers::lazy_request::refresh_thread_member_lists(
        state,
        &row.guild_id,
        thread_id,
    )
    .await;

    Ok(())
}

pub async fn add_thread_member(
    state: &AppState,
    thread_id: &str,
    actor_id: &str,
    target_user_id: &str,
    can_use_threads: bool,
) -> Result<(), AppError> {
    if target_user_id == "@me" {
        return join_thread(state, thread_id, actor_id, can_use_threads).await;
    }
    if !can_use_threads {
        return Err(AppError::not_found());
    }
    if validate_snowflake("user_id", target_user_id).is_err() {
        return Err(AppError::unknown_member());
    }

    let mut tx = state.db.begin().await?;
    let row = ensure_thread_row(&mut tx, thread_id).await?;
    let (actor_parent_perms, actor_thread_perms, _) =
        check_thread_view_access(&mut tx, &row, actor_id).await?;
    let actor_is_mod = actor_parent_perms.contains(PermissionFlags::MANAGE_THREADS);

    if !actor_thread_perms.contains(PermissionFlags::SEND_MESSAGES_IN_THREADS) {
        return Err(AppError::missing_permissions());
    }

    if !member_service::is_member(state, &row.guild_id, target_user_id).await? {
        return Err(AppError::unknown_member());
    }

    let target_parent_perms = permission_service::compute_user_thread_permissions_conn(
        &mut tx,
        &row.guild_id,
        &row.parent_id,
        target_user_id,
    )
    .await
    .map(|(p, _)| p)
    .unwrap_or_else(|_| PermissionFlags::empty());

    if !target_parent_perms.contains(PermissionFlags::VIEW_CHANNEL) {
        return Err(AppError::missing_access());
    }
    let target_is_mod = target_parent_perms.contains(PermissionFlags::MANAGE_THREADS);

    if row.kind == ChannelType::PrivateThread as u32
        && !row.invitable.unwrap_or(true)
        && !actor_is_mod
        && !target_is_mod
    {
        return Err(AppError::missing_permissions());
    }

    if row.archived {
        return Err(AppError::thread_archived());
    }
    if row.locked && !actor_is_mod {
        return Err(AppError::thread_locked());
    }

    let already = fetch_thread_member_self(&mut *tx, thread_id, target_user_id)
        .await?
        .is_some();
    if already {
        tx.commit().await?;
        return Ok(());
    }

    let current_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_members WHERE thread_id = ? FOR UPDATE")
            .bind(thread_id)
            .fetch_one(&mut *tx)
            .await?;
    if current_count >= MAX_THREAD_MEMBERS {
        return Err(AppError::max_thread_members());
    }

    let now = Utc::now().timestamp_millis() as u64;
    let (_, inserted_row, member_count) =
        add_thread_member_tx(&mut tx, thread_id, target_user_id, 0, now).await?;

    let sys_msg_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, timestamp, type) VALUES (?, ?, ?, '', ?, ?)",
    )
    .bind(&sys_msg_id)
    .bind(thread_id)
    .bind(actor_id)
    .bind(now)
    .bind(MessageType::RecipientAdd as u32)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT IGNORE INTO message_mentions (message_id, user_id) VALUES (?, ?)")
        .bind(&sys_msg_id)
        .bind(target_user_id)
        .execute(&mut *tx)
        .await?;

    let hydrated_for_target = hydrate_thread(&mut tx, row.clone(), target_user_id, false).await?;
    tx.commit().await?;

    let gm = fetch_guild_member_response(state, &row.guild_id, target_user_id).await;
    let pres = presence::build_presence_payloads(state, &[target_user_id.to_string()], None)
        .await
        .into_iter()
        .find(|p| p.status != "offline");
    let mut added = ThreadMember::from_db_public(inserted_row, None);
    added.member = Some(gm);
    added.presence = Some(pres);

    permission_service::dispatch_channel_with_ctx(
        state,
        &row.guild_id,
        &row.parent_id,
        None,
        events::THREAD_MEMBERS_UPDATE,
        &ThreadMembersUpdateEvent {
            id: thread_id.to_string(),
            guild_id: row.guild_id.clone(),
            member_count,
            added_members: Some(vec![added]),
            removed_member_ids: None,
        },
    )
    .await;

    state
        .dispatch_user(
            events::THREAD_CREATE,
            &ThreadCreateEvent {
                thread: hydrated_for_target,
                newly_created: None,
            },
            target_user_id,
        )
        .await;

    crate::gateway::handlers::lazy_request::refresh_thread_member_lists(
        state,
        &row.guild_id,
        thread_id,
    )
    .await;

    if let (Ok(author), Ok(target_user)) = (
        user::get_partial_user(state, actor_id).await,
        user::get_partial_user(state, target_user_id).await,
    ) {
        let sys_msg = Message {
            id: sys_msg_id,
            channel_id: thread_id.to_string(),
            author,
            kind: MessageType::RecipientAdd as u32,
            flags: 0,
            content: String::new(),
            timestamp: now,
            edited_timestamp: None,
            pinned: false,
            mention_everyone: false,
            tts: false,
            mentions: vec![target_user],
            mention_roles: vec![],
            embeds: vec![],
            attachments: vec![],
            stickers: vec![],
            message_reference: None,
            nonce: None,
            referenced_message: None,
            thread: None,
        };
        let actor_gm = fetch_guild_member_response(state, &row.guild_id, actor_id)
            .await
            .map(|r| GuildMemberFields::new(r.member.nick, r.member.roles, r.member.joined_at));
        permission_service::dispatch_channel_with_ctx(
            state,
            &row.guild_id,
            &row.parent_id,
            None,
            events::MESSAGE_CREATE,
            &GatewayMessageEvent {
                message: sys_msg,
                channel_type: row.kind,
                guild_id: Some(row.guild_id.clone()),
                member: actor_gm,
            },
        )
        .await;
    }

    Ok(())
}

pub async fn leave_thread(
    state: &AppState,
    thread_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<(), AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }

    let mut tx = state.db.begin().await?;
    let row = ensure_thread_row(&mut tx, thread_id).await?;
    let _ = check_thread_view_access(&mut tx, &row, user_id).await?;

    if fetch_thread_member_self(&mut *tx, thread_id, user_id)
        .await?
        .is_none()
    {
        return Err(AppError::unknown_thread_member());
    }
    if row.archived {
        return Err(AppError::thread_archived());
    }

    let member_count = remove_thread_member_tx(&mut tx, thread_id, user_id)
        .await?
        .ok_or_else(AppError::unknown_thread_member)?;
    tx.commit().await?;

    permission_service::dispatch_channel_with_ctx(
        state,
        &row.guild_id,
        &row.parent_id,
        None,
        events::THREAD_MEMBERS_UPDATE,
        &ThreadMembersUpdateEvent {
            id: thread_id.to_string(),
            guild_id: row.guild_id.clone(),
            member_count,
            added_members: None,
            removed_member_ids: Some(vec![user_id.to_string()]),
        },
    )
    .await;

    crate::gateway::handlers::lazy_request::refresh_thread_member_lists(
        state,
        &row.guild_id,
        thread_id,
    )
    .await;

    Ok(())
}

pub async fn remove_thread_member(
    state: &AppState,
    thread_id: &str,
    actor_id: &str,
    target_user_id: &str,
    can_use_threads: bool,
) -> Result<(), AppError> {
    if target_user_id == "@me" || target_user_id == actor_id {
        return leave_thread(state, thread_id, actor_id, can_use_threads).await;
    }
    if !can_use_threads {
        return Err(AppError::not_found());
    }
    if validate_snowflake("user_id", target_user_id).is_err() {
        return Err(AppError::unknown_thread_member());
    }

    let mut tx = state.db.begin().await?;
    let row = ensure_thread_row(&mut tx, thread_id).await?;
    let (actor_parent_perms, _, _) = check_thread_view_access(&mut tx, &row, actor_id).await?;
    let actor_is_mod = actor_parent_perms.contains(PermissionFlags::MANAGE_THREADS);
    let is_private_owner =
        row.kind == ChannelType::PrivateThread as u32 && row.owner_id == actor_id;

    if !actor_is_mod && !is_private_owner {
        return Err(AppError::missing_permissions());
    }
    if row.archived {
        return Err(AppError::thread_archived());
    }

    let member_count = remove_thread_member_tx(&mut tx, thread_id, target_user_id)
        .await?
        .ok_or_else(AppError::unknown_thread_member)?;

    let now = Utc::now().timestamp_millis() as u64;
    let sys_msg_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, timestamp, type) VALUES (?, ?, ?, '', ?, ?)",
    )
    .bind(&sys_msg_id)
    .bind(thread_id)
    .bind(actor_id)
    .bind(now)
    .bind(MessageType::RecipientRemove as u32)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT IGNORE INTO message_mentions (message_id, user_id) VALUES (?, ?)")
        .bind(&sys_msg_id)
        .bind(target_user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    permission_service::dispatch_channel_with_ctx(
        state,
        &row.guild_id,
        &row.parent_id,
        None,
        events::THREAD_MEMBERS_UPDATE,
        &ThreadMembersUpdateEvent {
            id: thread_id.to_string(),
            guild_id: row.guild_id.clone(),
            member_count,
            added_members: None,
            removed_member_ids: Some(vec![target_user_id.to_string()]),
        },
    )
    .await;

    crate::gateway::handlers::lazy_request::refresh_thread_member_lists(
        state,
        &row.guild_id,
        thread_id,
    )
    .await;

    if let (Ok(author), Ok(target_user)) = (
        user::get_partial_user(state, actor_id).await,
        user::get_partial_user(state, target_user_id).await,
    ) {
        let sys_msg = Message {
            id: sys_msg_id,
            channel_id: thread_id.to_string(),
            author,
            kind: MessageType::RecipientRemove as u32,
            flags: 0,
            content: String::new(),
            timestamp: now,
            edited_timestamp: None,
            pinned: false,
            mention_everyone: false,
            tts: false,
            mentions: vec![target_user],
            mention_roles: vec![],
            embeds: vec![],
            attachments: vec![],
            stickers: vec![],
            message_reference: None,
            nonce: None,
            referenced_message: None,
            thread: None,
        };
        let actor_gm = fetch_guild_member_response(state, &row.guild_id, actor_id)
            .await
            .map(|r| GuildMemberFields::new(r.member.nick, r.member.roles, r.member.joined_at));
        permission_service::dispatch_channel_with_ctx(
            state,
            &row.guild_id,
            &row.parent_id,
            None,
            events::MESSAGE_CREATE,
            &GatewayMessageEvent {
                message: sys_msg,
                channel_type: row.kind,
                guild_id: Some(row.guild_id.clone()),
                member: actor_gm,
            },
        )
        .await;
    }

    Ok(())
}

pub async fn get_thread_member(
    state: &AppState,
    thread_id: &str,
    actor_id: &str,
    target_user_id: &str,
    is_bot: bool,
    with_member: bool,
) -> Result<ThreadMember, AppError> {
    if !is_bot {
        return Err(AppError::not_found());
    }
    if validate_snowflake("user_id", target_user_id).is_err() {
        return Err(AppError::unknown_thread_member());
    }

    let mut conn = state.db.acquire().await?;
    let row = ensure_thread_row(&mut conn, thread_id).await?;
    let _ = check_thread_view_access(&mut conn, &row, actor_id).await?;

    let member_row = fetch_thread_member_self(&mut *conn, thread_id, target_user_id)
        .await?
        .ok_or_else(AppError::unknown_thread_member)?;
    drop(conn);

    let gm = if with_member {
        fetch_guild_member_response(state, &row.guild_id, target_user_id).await
    } else {
        None
    };

    if target_user_id == actor_id {
        let mut res = ThreadMember::from_db_self(member_row, true);
        res.member = gm.map(Some);
        Ok(res)
    } else {
        Ok(ThreadMember::from_db_public(member_row, gm))
    }
}

pub async fn list_thread_members(
    state: &AppState,
    thread_id: &str,
    actor_id: &str,
    is_bot: bool,
    query: ListThreadMembersQuery,
) -> Result<Vec<ThreadMember>, AppError> {
    if !is_bot {
        return Err(AppError::not_found());
    }
    validate_snowflake_opt("after", query.after.as_deref())?;

    let mut conn = state.db.acquire().await?;
    let row = ensure_thread_row(&mut conn, thread_id).await?;
    let _ = check_thread_view_access(&mut conn, &row, actor_id).await?;

    let limit = query
        .limit
        .unwrap_or(DEFAULT_THREAD_MEMBERS_LIMIT)
        .clamp(1, MAX_THREAD_MEMBERS_LIMIT);

    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT thread_id, user_id, join_timestamp, flags, muted, mute_config FROM thread_members WHERE thread_id = ",
    );
    qb.push_bind(thread_id);
    if let Some(after) = &query.after {
        qb.push(" AND CAST(user_id AS UNSIGNED) > CAST(");
        qb.push_bind(after);
        qb.push(" AS UNSIGNED)");
    }
    qb.push(" ORDER BY CAST(user_id AS UNSIGNED) ASC LIMIT ");
    qb.push_bind(limit);

    let member_rows = qb
        .build_query_as::<ThreadMemberRow>()
        .fetch_all(&mut *conn)
        .await?;
    drop(conn);

    let with_member = query.with_member.unwrap_or(false);
    let mut gm_by_user: HashMap<String, GuildMemberResponse> = HashMap::new();
    if with_member && !member_rows.is_empty() {
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
        mqb.push_bind(&row.guild_id);
        mqb.push(" AND gm.user_id IN (");
        let mut sep = mqb.separated(", ");
        for m in &member_rows {
            sep.push_bind(&m.user_id);
        }
        mqb.push(")");
        let gm_rows = mqb
            .build_query_as::<GuildMemberUserRow>()
            .fetch_all(&state.db)
            .await?;
        let mut roles_by_user =
            queries::fetch_guild_member_roles_map(&state.db, &row.guild_id).await?;
        for mr in gm_rows {
            let roles = roles_by_user.remove(&mr.user_id).unwrap_or_default();
            let uid = mr.user_id.clone();
            gm_by_user.insert(uid, mr.into_member_response(roles));
        }
    }

    Ok(member_rows
        .into_iter()
        .map(|m| {
            let gm = gm_by_user.remove(&m.user_id);
            ThreadMember::from_db_public(m, gm)
        })
        .collect())
}

pub async fn modify_thread_member_settings(
    state: &AppState,
    thread_id: &str,
    user_id: &str,
    is_bot: bool,
    payload: ModifyThreadMemberSettingsRequest,
) -> Result<Option<ThreadMember>, AppError> {
    if is_bot {
        return Err(AppError::access_denied());
    }

    let mut tx = state.db.begin().await?;
    let row = ensure_thread_row(&mut tx, thread_id).await?;
    let _ = check_thread_view_access(&mut tx, &row, user_id).await?;

    let current = fetch_thread_member_self(&mut *tx, thread_id, user_id)
        .await?
        .ok_or_else(AppError::unknown_thread_member)?;

    let mut next_flags = current.flags;
    if let Some(req_flags) = payload.flags {
        let notif_mask = ThreadMemberFlags::NOTIFICATION_MASK.bits();
        if (req_flags & !notif_mask) != 0 || req_flags.count_ones() > 1 {
            return Err(AppError::invalid_thread_notification_settings());
        }
        next_flags = (current.flags & ThreadMemberFlags::HAS_INTERACTED.bits()) | req_flags;
    }

    let next_muted = payload.muted.unwrap_or(current.muted);
    let next_mute_config = match payload.mute_config {
        Some(Some(val)) => Some(
            serde_json::to_string(&val)
                .map_err(|e| AppError::internal(format!("Failed to serialize mute_config: {e}")))?,
        ),
        Some(None) => None,
        None => current.mute_config.clone(),
    };

    if next_flags == current.flags
        && next_muted == current.muted
        && next_mute_config == current.mute_config
    {
        tx.commit().await?;
        return Ok(None);
    }

    sqlx::query(
        "UPDATE thread_members SET flags = ?, muted = ?, mute_config = ? WHERE thread_id = ? AND user_id = ?",
    )
    .bind(next_flags)
    .bind(next_muted)
    .bind(&next_mute_config)
    .bind(thread_id)
    .bind(user_id)
    .execute(&mut *tx)
    .await?;

    let updated_row = ThreadMemberRow {
        thread_id: current.thread_id,
        user_id: current.user_id,
        join_timestamp: current.join_timestamp,
        flags: next_flags,
        muted: next_muted,
        mute_config: next_mute_config,
    };
    tx.commit().await?;

    let member = ThreadMember::from_db_self(updated_row, true);
    state
        .dispatch_user(
            events::THREAD_MEMBER_UPDATE,
            &ThreadMemberUpdateEvent {
                guild_id: row.guild_id,
                member: member.clone(),
            },
            user_id,
        )
        .await;

    Ok(Some(member))
}
