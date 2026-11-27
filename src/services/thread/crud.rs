use std::collections::HashSet;

use chrono::Utc;
use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::{ChannelType, DEFAULT_AUTO_ARCHIVE_DURATION};
use crate::constants::flags::{ChannelFlags, MessageFlags, PermissionFlags, ThreadMemberFlags};
use crate::constants::gateway::events;
use crate::constants::limits::MAX_MESSAGE_LENGTH;
use crate::constants::message::MessageType;
use crate::error::{AppError, ValidationCode};
use crate::models::gateway::events::{GatewayMessageEvent, ThreadCreateEvent, ThreadDeleteEvent};
use crate::models::guild_member::GuildMemberFields;
use crate::models::message::{Message, MessageRow};
use crate::models::rest::channels::UpdateChannelRequest;
use crate::models::rest::threads::{StartThreadFromMessageRequest, StartThreadRequest};
use crate::models::thread::{ForumPostResponse, StartThreadResponse, ThreadResponse, ThreadRow};
use crate::services::{permission_service, queries, user};
use crate::utils::snowflake::DISCORD_SNOWFLAKE;

use super::helpers::{
    add_thread_member_tx, check_thread_view_access, ensure_active_thread_capacity,
    fetch_thread_row, hydrate_thread, sync_thread_applied_tags, validate_and_prepare_applied_tags,
    validate_auto_archive_duration_val, validate_thread_name, validate_thread_rate_limit,
};

async fn fetch_message_member_fields(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Option<GuildMemberFields> {
    let row: Option<(Option<String>, u64)> = sqlx::query_as(
        "SELECT nick, joined_at FROM guild_members WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let (nick, joined_at) = row?;
    let roles = queries::fetch_member_roles(&state.db, guild_id, user_id)
        .await
        .unwrap_or_default();
    Some(GuildMemberFields::new(nick, roles, joined_at))
}

pub async fn get_thread(
    state: &AppState,
    thread_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<ThreadResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::unknown_channel());
    }
    let mut conn = state.db.acquire().await?;
    let row = fetch_thread_row(&mut *conn, thread_id)
        .await?
        .ok_or_else(AppError::unknown_channel)?;

    let _ = check_thread_view_access(&mut conn, &row, user_id).await?;
    hydrate_thread(&mut conn, row, user_id, true).await
}

pub async fn start_thread_from_message(
    state: &AppState,
    channel_id: &str,
    message_id: &str,
    user_id: &str,
    can_use_threads: bool,
    is_bot: bool,
    payload: StartThreadFromMessageRequest,
) -> Result<ThreadResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }

    let name = validate_thread_name(&payload.name)?;
    let auto_archive_duration = validate_auto_archive_duration_val(
        payload
            .auto_archive_duration
            .unwrap_or(DEFAULT_AUTO_ARCHIVE_DURATION),
    )?;

    let parent: Option<(String, u32, u32)> = sqlx::query_as(
        "SELECT guild_id, type, default_thread_rate_limit_per_user FROM channels WHERE id = ? AND type NOT IN (10, 11, 12)",
    )
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?;

    let Some((guild_id, parent_kind, default_rate_limit)) = parent else {
        return Err(AppError::not_found());
    };

    if parent_kind != ChannelType::GuildText as u32
        && parent_kind != ChannelType::GuildAnnouncement as u32
    {
        return Err(AppError::invalid_channel_type());
    }

    let (perm_ctx, _, parent_perms) = permission_service::require_channel_permission_with_ctx(
        state,
        &guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL
            | PermissionFlags::CREATE_PUBLIC_THREADS
            | PermissionFlags::READ_MESSAGE_HISTORY,
    )
    .await?;

    let rate_limit_per_user =
        validate_thread_rate_limit(payload.rate_limit_per_user.unwrap_or(default_rate_limit))?;

    let thread_kind = if parent_kind == ChannelType::GuildAnnouncement as u32 {
        ChannelType::AnnouncementThread as u32
    } else {
        ChannelType::PublicThread as u32
    };

    let mut tx = state.db.begin().await?;

    let _: Option<(String,)> = sqlx::query_as("SELECT id FROM guilds WHERE id = ? FOR UPDATE")
        .bind(&guild_id)
        .fetch_optional(&mut *tx)
        .await?;

    let msg_row = sqlx::query_as::<_, MessageRow>(&format!(
        "{} WHERE m.id = ? AND m.channel_id = ? FOR UPDATE",
        MessageRow::SELECT_FROM
    ))
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(AppError::unknown_message)?;

    if msg_row.kind != MessageType::Default as u32 && msg_row.kind != MessageType::Reply as u32 {
        return Err(AppError::coded(
            axum::http::StatusCode::BAD_REQUEST,
            crate::error::ApiErrorCode::CannotModifySystemWebhook,
            "Cannot start a thread from a system message.",
        ));
    }

    let already_exists: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM channel_threads WHERE channel_id = ? LIMIT 1")
            .bind(message_id)
            .fetch_optional(&mut *tx)
            .await?;
    if already_exists.is_some() {
        return Err(AppError::thread_already_created_for_message());
    }

    let newest_five: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM messages WHERE channel_id = ? ORDER BY id_num DESC LIMIT 5",
    )
    .bind(channel_id)
    .fetch_all(&mut *tx)
    .await?;
    let emit_system_msg = !newest_five.iter().any(|id| id == message_id);

    ensure_active_thread_capacity(&mut tx, &guild_id).await?;

    let now = Utc::now().timestamp_millis() as u64;

    sqlx::query(
        r#"
        INSERT INTO channels (id, type, position, guild_id, parent_id, name, rate_limit_per_user, flags)
        VALUES (?, ?, 0, ?, ?, ?, ?, 0)
        "#,
    )
    .bind(message_id)
    .bind(thread_kind)
    .bind(&guild_id)
    .bind(channel_id)
    .bind(&name)
    .bind(rate_limit_per_user)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO channel_threads (
            channel_id, guild_id, parent_id, owner_id, started_from_message,
            archived, auto_archive_duration, archive_timestamp, locked, invitable,
            create_timestamp, message_count, total_message_sent, member_count
        ) VALUES (?, ?, ?, ?, TRUE, FALSE, ?, ?, FALSE, NULL, ?, 0, 0, 0)
        "#,
    )
    .bind(message_id)
    .bind(&guild_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(auto_archive_duration)
    .bind(now)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    let member_flags = if is_bot {
        0
    } else {
        ThreadMemberFlags::HAS_INTERACTED.bits()
    };
    add_thread_member_tx(&mut tx, message_id, user_id, member_flags, now).await?;

    let new_msg_flags = msg_row.flags | MessageFlags::HAS_THREAD.bits();
    sqlx::query("UPDATE messages SET flags = ? WHERE id = ?")
        .bind(new_msg_flags)
        .bind(message_id)
        .execute(&mut *tx)
        .await?;

    let sys_msg_id = if emit_system_msg {
        let sid = DISCORD_SNOWFLAKE.generate(None).to_string();
        sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, content, timestamp, type) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&sid)
        .bind(channel_id)
        .bind(user_id)
        .bind(&name)
        .bind(now)
        .bind(MessageType::ThreadCreated as u32)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
            .bind(&sid)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        Some(sid)
    } else {
        None
    };

    if let Some(ref sid) = sys_msg_id
        && !is_bot
    {
        crate::services::read_state_service::mark_channel_read_on_send_tx(
            &mut tx, user_id, channel_id, sid,
        )
        .await?;
    }

    let row = fetch_thread_row(&mut *tx, message_id)
        .await?
        .ok_or_else(AppError::unknown_channel)?;
    let thread = hydrate_thread(&mut tx, row, user_id, true).await?;

    let _ = parent_perms;
    tx.commit().await?;

    let create_evt = ThreadCreateEvent {
        thread: thread.clone(),
        newly_created: Some(true),
    };
    permission_service::dispatch_channel_with_ctx(
        state,
        &guild_id,
        channel_id,
        perm_ctx.as_ref(),
        events::THREAD_CREATE,
        &create_evt,
    )
    .await;

    let mut hydrated_src =
        crate::services::message_service::hydrate_messages(state, vec![msg_row], user_id, true)
            .await?;
    if let Some(mut updated_source_msg) = hydrated_src.pop() {
        updated_source_msg.flags = new_msg_flags;
        updated_source_msg.thread = Some(Box::new(thread.clone()));
        let member =
            fetch_message_member_fields(state, &guild_id, &updated_source_msg.author.id).await;
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            channel_id,
            perm_ctx.as_ref(),
            events::MESSAGE_UPDATE,
            &GatewayMessageEvent {
                message: updated_source_msg,
                channel_type: parent_kind,
                guild_id: Some(guild_id.clone()),
                member,
            },
        )
        .await;
    }

    let ack_sys_msg_id = (!is_bot).then_some(sys_msg_id.clone()).flatten();
    if let Some(sid) = sys_msg_id
        && let Ok(author) = user::get_partial_user(state, user_id).await
    {
        let sys_msg = Message {
            id: sid,
            channel_id: channel_id.to_string(),
            author,
            kind: MessageType::ThreadCreated as u32,
            flags: 0,
            content: name,
            timestamp: now,
            edited_timestamp: None,
            pinned: false,
            mention_everyone: false,
            tts: false,
            mentions: vec![],
            mention_roles: vec![],
            embeds: vec![],
            attachments: vec![],
            stickers: vec![],
            message_reference: None,
            nonce: None,
            referenced_message: None,
            thread: Some(Box::new(thread.clone())),
        };
        let actor_member = fetch_message_member_fields(state, &guild_id, user_id).await;
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            channel_id,
            perm_ctx.as_ref(),
            events::MESSAGE_CREATE,
            &GatewayMessageEvent {
                message: sys_msg,
                channel_type: parent_kind,
                guild_id: Some(guild_id.clone()),
                member: actor_member,
            },
        )
        .await;
    }

    if let Some(ref sid) = ack_sys_msg_id {
        crate::services::read_state_service::dispatch_message_ack(
            state,
            user_id,
            parent_kind,
            channel_id,
            sid,
            0,
            false,
        )
        .await;
    }

    Ok(thread)
}

pub async fn start_thread(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    is_bot: bool,
    payload: StartThreadRequest,
) -> Result<StartThreadResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }

    let name = validate_thread_name(&payload.name)?;
    let auto_archive_duration = validate_auto_archive_duration_val(
        payload
            .auto_archive_duration
            .unwrap_or(DEFAULT_AUTO_ARCHIVE_DURATION),
    )?;

    let parent: Option<(String, u32, u32, u32, u32)> = sqlx::query_as(
        "SELECT guild_id, type, flags, rate_limit_per_user, default_thread_rate_limit_per_user FROM channels WHERE id = ? AND type NOT IN (10, 11, 12)",
    )
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?;

    let Some((guild_id, parent_kind, parent_flags, parent_rate_limit, default_thread_rate_limit)) =
        parent
    else {
        return Err(AppError::not_found());
    };

    let is_forum_like = ChannelType::is_forum_like(parent_kind);
    let thread_kind = if is_forum_like {
        if let Some(req_kind) = payload.kind
            && req_kind != ChannelType::PublicThread as u32
        {
            return Err(AppError::invalid_channel_type());
        }
        ChannelType::PublicThread as u32
    } else if parent_kind == ChannelType::GuildAnnouncement as u32 {
        match payload.kind {
            Some(k) if k == ChannelType::AnnouncementThread as u32 => k,
            _ => return Err(AppError::invalid_channel_type()),
        }
    } else if parent_kind == ChannelType::GuildText as u32 {
        match payload.kind {
            Some(k)
                if k == ChannelType::PublicThread as u32
                    || k == ChannelType::PrivateThread as u32 =>
            {
                k
            }
            _ => return Err(AppError::invalid_channel_type()),
        }
    } else {
        return Err(AppError::invalid_channel_type());
    };

    let required_perm = if is_forum_like {
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::SEND_MESSAGES
    } else if thread_kind == ChannelType::PrivateThread as u32 {
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::CREATE_PRIVATE_THREADS
    } else {
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::CREATE_PUBLIC_THREADS
    };

    let (perm_ctx, member_roles, parent_perms) =
        permission_service::require_channel_permission_with_ctx(
            state,
            &guild_id,
            channel_id,
            user_id,
            required_perm,
        )
        .await?;

    let is_moderator = parent_perms.contains(PermissionFlags::MANAGE_THREADS);
    let rate_limit_per_user = validate_thread_rate_limit(
        payload
            .rate_limit_per_user
            .unwrap_or(default_thread_rate_limit),
    )?;

    let first_msg_data = if is_forum_like {
        let msg_input = payload.message.ok_or_else(|| {
            AppError::validation(
                "message",
                ValidationCode::InvalidFormat,
                "message is required to create a forum post.",
            )
        })?;
        let content = msg_input.content.unwrap_or_default();
        if content.trim().is_empty() {
            return Err(AppError::empty_message());
        }
        if content.chars().count() > MAX_MESSAGE_LENGTH {
            return Err(AppError::validation(
                "message.content",
                ValidationCode::ContentExceedsMaxLength,
                format!("Text must not exceed {MAX_MESSAGE_LENGTH} characters."),
            ));
        }
        let allowed_msg_flags =
            MessageFlags::SUPPRESS_EMBEDS.bits() | MessageFlags::SUPPRESS_NOTIFICATIONS.bits();
        let msg_flags = msg_input.flags.unwrap_or(0) & allowed_msg_flags;
        let resolved_mentions = crate::services::mention_service::resolve_mentions(
            state,
            &guild_id,
            user_id,
            parent_perms,
            &content,
            msg_input.allowed_mentions.as_ref(),
            None,
        )
        .await?;
        Some((content, msg_flags, resolved_mentions))
    } else {
        None
    };

    let now = Utc::now().timestamp_millis() as u64;
    if is_forum_like
        && parent_rate_limit > 0
        && !is_bot
        && !parent_perms.contains(PermissionFlags::BYPASS_SLOWMODE)
    {
        let last_post: Option<u64> = sqlx::query_scalar(
            "SELECT MAX(create_timestamp) FROM channel_threads WHERE parent_id = ? AND owner_id = ?",
        )
        .bind(channel_id)
        .bind(user_id)
        .fetch_one(&state.db)
        .await?;
        if let Some(last_ts) = last_post
            && now.saturating_sub(last_ts) < (parent_rate_limit as u64) * 1000
        {
            return Err(AppError::slowmode_rate_limited());
        }
    }

    let mut tx = state.db.begin().await?;

    let _: Option<(String,)> = sqlx::query_as("SELECT id FROM guilds WHERE id = ? FOR UPDATE")
        .bind(&guild_id)
        .fetch_optional(&mut *tx)
        .await?;

    ensure_active_thread_capacity(&mut tx, &guild_id).await?;

    let applied_tags = if is_forum_like {
        validate_and_prepare_applied_tags(
            &mut tx,
            channel_id,
            parent_flags,
            payload.applied_tags.as_deref(),
            None,
            is_moderator,
        )
        .await?
    } else {
        Vec::new()
    };

    let thread_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let invitable = if thread_kind == ChannelType::PrivateThread as u32 {
        Some(payload.invitable.unwrap_or(true))
    } else {
        None
    };

    sqlx::query(
        r#"
        INSERT INTO channels (id, type, position, guild_id, parent_id, name, last_message_id, rate_limit_per_user, flags)
        VALUES (?, ?, 0, ?, ?, ?, ?, ?, 0)
        "#,
    )
    .bind(&thread_id)
    .bind(thread_kind)
    .bind(&guild_id)
    .bind(channel_id)
    .bind(&name)
    .bind(first_msg_data.as_ref().map(|_| thread_id.as_str()))
    .bind(rate_limit_per_user)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO channel_threads (
            channel_id, guild_id, parent_id, owner_id, started_from_message,
            archived, auto_archive_duration, archive_timestamp, locked, invitable,
            create_timestamp, message_count, total_message_sent, member_count
        ) VALUES (?, ?, ?, ?, FALSE, FALSE, ?, ?, FALSE, ?, ?, 0, 0, 0)
        "#,
    )
    .bind(&thread_id)
    .bind(&guild_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(auto_archive_duration)
    .bind(now)
    .bind(invitable)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    if !applied_tags.is_empty() {
        sync_thread_applied_tags(&mut tx, &thread_id, &applied_tags).await?;
    }

    let member_flags = if is_bot {
        0
    } else {
        ThreadMemberFlags::HAS_INTERACTED.bits()
    };
    add_thread_member_tx(&mut tx, &thread_id, user_id, member_flags, now).await?;

    if let Some((ref content, msg_flags, ref resolved)) = first_msg_data {
        sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, content, timestamp, type, flags, mention_everyone) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&thread_id)
        .bind(&thread_id)
        .bind(user_id)
        .bind(content)
        .bind(now)
        .bind(MessageType::Default as u32)
        .bind(msg_flags)
        .bind(resolved.mention_everyone)
        .execute(&mut *tx)
        .await?;

        crate::services::mention_service::persist_mentions_tx(
            &mut tx,
            &thread_id,
            &resolved.mentions,
            &resolved.mention_roles,
            false,
        )
        .await?;

        if (msg_flags & MessageFlags::SUPPRESS_NOTIFICATIONS.bits()) == 0 {
            crate::services::mention_service::increment_mention_counts_tx(
                &mut tx,
                &guild_id,
                &thread_id,
                thread_kind,
                Some(channel_id),
                user_id,
                resolved,
            )
            .await?;
        }

        sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
            .bind(&thread_id)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
    }

    let sys_msg_id = if !is_forum_like && thread_kind != ChannelType::PrivateThread as u32 {
        let sid = DISCORD_SNOWFLAKE.generate(None).to_string();
        sqlx::query(
            "INSERT INTO messages (id, channel_id, author_id, content, timestamp, type) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&sid)
        .bind(channel_id)
        .bind(user_id)
        .bind(&name)
        .bind(now)
        .bind(MessageType::ThreadCreated as u32)
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
            .bind(&sid)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        Some(sid)
    } else {
        None
    };

    if !is_bot {
        if first_msg_data.is_some() {
            crate::services::read_state_service::mark_channel_read_on_send_tx(
                &mut tx, user_id, &thread_id, &thread_id,
            )
            .await?;
            crate::services::read_state_service::mark_channel_read_on_send_tx(
                &mut tx, user_id, channel_id, &thread_id,
            )
            .await?;
        } else if let Some(ref sid) = sys_msg_id {
            crate::services::read_state_service::mark_channel_read_on_send_tx(
                &mut tx, user_id, channel_id, sid,
            )
            .await?;
        }
    }

    let row = fetch_thread_row(&mut *tx, &thread_id)
        .await?
        .ok_or_else(AppError::unknown_channel)?;
    let thread = hydrate_thread(&mut tx, row, user_id, true).await?;

    tx.commit().await?;

    let create_evt = ThreadCreateEvent {
        thread: thread.clone(),
        newly_created: Some(true),
    };
    if thread_kind == ChannelType::PrivateThread as u32 {
        let sessions = permission_service::filter_sessions_for_channel_with_ctx(
            state,
            &guild_id,
            channel_id,
            perm_ctx.as_ref(),
        )
        .await;
        // Private thread creation goes to creator and moderators who can view it.
        let mut allowed_sessions = Vec::new();
        for s in sessions {
            let Some(uid) = s.user_id() else { continue };
            if uid == user_id {
                allowed_sessions.push(s);
            } else if let Ok((p_perms, _)) = async {
                let mut c = state.db.acquire().await?;
                permission_service::compute_user_thread_permissions_conn(
                    &mut c, &guild_id, channel_id, &uid,
                )
                .await
            }
            .await
                && p_perms.contains(PermissionFlags::MANAGE_THREADS)
            {
                allowed_sessions.push(s);
            }
        }
        state.dispatch_to_sessions(&allowed_sessions, events::THREAD_CREATE, &create_evt);
    } else {
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            channel_id,
            perm_ctx.as_ref(),
            events::THREAD_CREATE,
            &create_evt,
        )
        .await;
    }

    if let Some((content, msg_flags, resolved)) = first_msg_data {
        let author = match state.hub.get_cached_partial_user(user_id) {
            Some(u) => u,
            None => user::get_partial_user(state, user_id).await?,
        };
        let first_message = Message {
            id: thread_id.clone(),
            channel_id: thread_id.clone(),
            author,
            kind: MessageType::Default as u32,
            flags: msg_flags,
            content,
            timestamp: now,
            edited_timestamp: None,
            pinned: false,
            mention_everyone: resolved.mention_everyone,
            tts: false,
            mentions: resolved.mentions,
            mention_roles: resolved.mention_roles,
            embeds: vec![],
            attachments: vec![],
            stickers: vec![],
            message_reference: None,
            nonce: None,
            referenced_message: None,
            thread: None,
        };
        let member = match member_roles {
            Some(roles) => {
                let row: Option<(Option<String>, u64)> = sqlx::query_as(
                    "SELECT nick, joined_at FROM guild_members WHERE guild_id = ? AND user_id = ?",
                )
                .bind(&guild_id)
                .bind(user_id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten();
                row.map(|(nick, joined_at)| GuildMemberFields::new(nick, roles, joined_at))
            }
            None => fetch_message_member_fields(state, &guild_id, user_id).await,
        };
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            channel_id,
            perm_ctx.as_ref(),
            events::MESSAGE_CREATE,
            &GatewayMessageEvent {
                message: first_message.clone(),
                channel_type: thread_kind,
                guild_id: Some(guild_id.clone()),
                member,
            },
        )
        .await;
        if !is_bot {
            crate::services::read_state_service::dispatch_message_ack(
                state,
                user_id,
                thread_kind,
                &thread_id,
                &thread_id,
                0,
                false,
            )
            .await;
            crate::services::read_state_service::dispatch_message_ack(
                state,
                user_id,
                parent_kind,
                channel_id,
                &thread_id,
                0,
                false,
            )
            .await;
        }
        return Ok(StartThreadResponse::ForumPost(Box::new(
            ForumPostResponse {
                thread,
                message: first_message,
            },
        )));
    }

    let ack_sys_msg_id = (!is_bot).then_some(sys_msg_id.clone()).flatten();
    if let Some(sid) = sys_msg_id
        && let Ok(author) = user::get_partial_user(state, user_id).await
    {
        let sys_msg = Message {
            id: sid,
            channel_id: channel_id.to_string(),
            author,
            kind: MessageType::ThreadCreated as u32,
            flags: 0,
            content: name,
            timestamp: now,
            edited_timestamp: None,
            pinned: false,
            mention_everyone: false,
            tts: false,
            mentions: vec![],
            mention_roles: vec![],
            embeds: vec![],
            attachments: vec![],
            stickers: vec![],
            message_reference: None,
            nonce: None,
            referenced_message: None,
            thread: Some(Box::new(thread.clone())),
        };
        let actor_member = fetch_message_member_fields(state, &guild_id, user_id).await;
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            channel_id,
            perm_ctx.as_ref(),
            events::MESSAGE_CREATE,
            &GatewayMessageEvent {
                message: sys_msg,
                channel_type: parent_kind,
                guild_id: Some(guild_id.clone()),
                member: actor_member,
            },
        )
        .await;
    }

    if let Some(ref sid) = ack_sys_msg_id {
        crate::services::read_state_service::dispatch_message_ack(
            state,
            user_id,
            parent_kind,
            channel_id,
            sid,
            0,
            false,
        )
        .await;
    }

    Ok(StartThreadResponse::Thread(Box::new(thread)))
}

pub async fn update_thread(
    state: &AppState,
    thread_id: &str,
    user_id: &str,
    payload: UpdateChannelRequest,
) -> Result<ThreadResponse, AppError> {
    let mut tx = state.db.begin().await?;
    let sql = format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE c.id = ? FOR UPDATE",
        ThreadRow::SELECT_COLS
    );
    let current = sqlx::query_as::<_, ThreadRow>(&sql)
        .bind(thread_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_channel)?;

    let (parent_perms, _, is_member) = check_thread_view_access(&mut tx, &current, user_id).await?;
    let is_moderator = parent_perms.contains(PermissionFlags::MANAGE_THREADS);
    let is_owner = current.owner_id == user_id;
    let is_forum_post = ChannelType::is_forum_like(current.parent_kind);

    if current.locked && !is_moderator {
        return Err(AppError::thread_locked());
    }

    let req_archived = payload.archived.flatten();
    let req_locked = payload.locked.flatten();
    let req_invitable = payload.invitable.flatten();
    let req_rate_limit = payload.rate_limit_per_user.map(|o| o.unwrap_or(0));

    let wants_mod_only =
        req_rate_limit.is_some() || payload.flags.is_some() || req_locked == Some(false);
    if wants_mod_only && !is_moderator {
        return Err(AppError::missing_permissions());
    }

    let wants_owner_or_mod = payload.name.is_some()
        || payload.auto_archive_duration.is_some()
        || req_invitable.is_some()
        || payload.applied_tags.is_some()
        || req_archived == Some(true)
        || req_locked == Some(true);
    if wants_owner_or_mod && !is_owner && !is_moderator {
        return Err(AppError::missing_permissions());
    }

    if req_archived == Some(false) && !is_member && !is_moderator {
        return Err(AppError::missing_permissions());
    }

    let mut ch_qb = QueryBuilder::<MySql>::new("UPDATE channels SET ");
    let mut ch_sep = ch_qb.separated(", ");
    let mut has_ch_updates = false;

    let mut ct_qb = QueryBuilder::<MySql>::new("UPDATE channel_threads SET ");
    let mut ct_sep = ct_qb.separated(", ");
    let mut has_ct_updates = false;

    if let Some(raw_name) = &payload.name {
        let name = validate_thread_name(raw_name)?;
        if name != current.name {
            ch_sep.push("name = ");
            ch_sep.push_bind_unseparated(name);
            has_ch_updates = true;
        }
    }

    if let Some(rate) = req_rate_limit {
        let val = validate_thread_rate_limit(rate)?;
        if val != current.rate_limit_per_user {
            ch_sep.push("rate_limit_per_user = ");
            ch_sep.push_bind_unseparated(val);
            has_ch_updates = true;
        }
    }

    let mut next_flags = current.flags;
    if let Some(flags_val) = payload.flags {
        if !is_forum_post || (flags_val & !ChannelFlags::THREAD_ALLOWED.bits()) != 0 {
            return Err(AppError::validation(
                "flags",
                ValidationCode::InvalidFormat,
                "Invalid thread flags.",
            ));
        }
        if (flags_val & ChannelFlags::PINNED.bits()) != 0
            && (current.flags & ChannelFlags::PINNED.bits()) == 0
        {
            let other_pinned: Option<(i32,)> = sqlx::query_as(
                "SELECT 1 FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id WHERE ct.parent_id = ? AND c.id != ? AND (c.flags & ?) != 0 LIMIT 1",
            )
            .bind(&current.parent_id)
            .bind(thread_id)
            .bind(ChannelFlags::PINNED.bits())
            .fetch_optional(&mut *tx)
            .await?;
            if other_pinned.is_some() {
                return Err(AppError::max_pinned_threads_in_forum());
            }
        }
        next_flags = flags_val;
    }

    let now = Utc::now().timestamp_millis() as u64;
    let mut bump_archive_ts = false;

    if let Some(archived) = req_archived
        && archived != current.archived
    {
        if !archived {
            ensure_active_thread_capacity(&mut tx, &current.guild_id).await?;
        } else {
            next_flags &= !ChannelFlags::PINNED.bits();
        }
        ct_sep.push("archived = ");
        ct_sep.push_bind_unseparated(archived);
        has_ct_updates = true;
        bump_archive_ts = true;
    }

    if next_flags != current.flags {
        ch_sep.push("flags = ");
        ch_sep.push_bind_unseparated(next_flags);
        has_ch_updates = true;
    }

    if let Some(duration) = payload.auto_archive_duration {
        let val = validate_auto_archive_duration_val(duration)?;
        if val != current.auto_archive_duration {
            ct_sep.push("auto_archive_duration = ");
            ct_sep.push_bind_unseparated(val);
            has_ct_updates = true;
            bump_archive_ts = true;
        }
    }

    if bump_archive_ts {
        ct_sep.push("archive_timestamp = ");
        ct_sep.push_bind_unseparated(now);
        has_ct_updates = true;
    }

    if let Some(locked) = req_locked
        && locked != current.locked
    {
        ct_sep.push("locked = ");
        ct_sep.push_bind_unseparated(locked);
        has_ct_updates = true;
    }

    if current.kind == ChannelType::PrivateThread as u32
        && let Some(invitable) = req_invitable
        && current.invitable != Some(invitable)
    {
        ct_sep.push("invitable = ");
        ct_sep.push_bind_unseparated(Some(invitable));
        has_ct_updates = true;
    }

    let mut tags_changed = false;
    if is_forum_post && let Some(ref tag_inputs) = payload.applied_tags {
        let (parent_flags,): (u32,) = sqlx::query_as("SELECT flags FROM channels WHERE id = ?")
            .bind(&current.parent_id)
            .fetch_one(&mut *tx)
            .await?;
        let current_tags: Vec<String> = sqlx::query_scalar(
            "SELECT tag_id FROM thread_applied_tags WHERE thread_id = ? ORDER BY position ASC",
        )
        .bind(thread_id)
        .fetch_all(&mut *tx)
        .await?;
        let next_tags = validate_and_prepare_applied_tags(
            &mut tx,
            &current.parent_id,
            parent_flags,
            Some(tag_inputs),
            Some(&current_tags),
            is_moderator,
        )
        .await?;
        let old_set: HashSet<&String> = current_tags.iter().collect();
        let new_set: HashSet<&String> = next_tags.iter().collect();
        if current_tags != next_tags || old_set != new_set {
            sync_thread_applied_tags(&mut tx, thread_id, &next_tags).await?;
            tags_changed = true;
        }
    }

    if has_ch_updates {
        ch_qb.push(" WHERE id = ");
        ch_qb.push_bind(thread_id);
        ch_qb.build().execute(&mut *tx).await?;
    }
    if has_ct_updates {
        ct_qb.push(" WHERE channel_id = ");
        ct_qb.push_bind(thread_id);
        ct_qb.build().execute(&mut *tx).await?;
    }

    let updated_row = fetch_thread_row(&mut *tx, thread_id)
        .await?
        .ok_or_else(AppError::unknown_channel)?;
    let updated = hydrate_thread(&mut tx, updated_row, user_id, true).await?;

    tx.commit().await?;

    if has_ch_updates || has_ct_updates || tags_changed {
        let mut broadcast = updated.clone();
        broadcast.member = None;
        permission_service::dispatch_channel_with_ctx(
            state,
            &current.guild_id,
            &current.parent_id,
            None,
            events::THREAD_UPDATE,
            &broadcast,
        )
        .await;
    }

    Ok(updated)
}

pub async fn delete_thread(
    state: &AppState,
    thread_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let mut conn = state.db.acquire().await?;
    let current = fetch_thread_row(&mut *conn, thread_id)
        .await?
        .ok_or_else(AppError::unknown_channel)?;

    let (parent_perms, _, _) = check_thread_view_access(&mut conn, &current, user_id).await?;
    if !parent_perms.contains(PermissionFlags::MANAGE_THREADS) {
        return Err(AppError::missing_permissions());
    }
    drop(conn);

    let sessions = permission_service::filter_sessions_for_channel_with_ctx(
        state,
        &current.guild_id,
        &current.parent_id,
        None,
    )
    .await;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(thread_id)
        .execute(&mut *tx)
        .await?;

    if ChannelType::is_forum_like(current.parent_kind) {
        sqlx::query(
            r#"
            UPDATE channels
            SET last_message_id = (
                SELECT ct.channel_id FROM channel_threads ct
                WHERE ct.parent_id = ?
                ORDER BY CAST(ct.channel_id AS UNSIGNED) DESC
                LIMIT 1
            )
            WHERE id = ? AND last_message_id = ?
            "#,
        )
        .bind(&current.parent_id)
        .bind(&current.parent_id)
        .bind(thread_id)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    state.dispatch_to_sessions(
        &sessions,
        events::THREAD_DELETE,
        &ThreadDeleteEvent {
            id: &current.id,
            guild_id: &current.guild_id,
            parent_id: &current.parent_id,
            kind: current.kind,
        },
    );

    Ok(())
}
