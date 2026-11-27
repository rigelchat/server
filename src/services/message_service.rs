use std::collections::{HashMap, HashSet};

use chrono::Utc;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::{MessageFlags, PermissionFlags};
use crate::constants::gateway::events;
use crate::constants::limits::{
    DEFAULT_MESSAGES_PAGE_LIMIT, MAX_MESSAGE_LENGTH, MAX_MESSAGE_NONCE_LENGTH,
    MAX_MESSAGES_PAGE_LIMIT,
};
use crate::constants::message::MessageType;
use crate::error::{AppError, ValidationCode};
use crate::models::gateway::events::{GatewayMessageEvent, MessageDeleteEvent, MessageMember};
use crate::models::message::{Message, MessageReference, MessageReferenceType, MessageRow};
use crate::models::rest::messages::{CreateMessageRequest, EditMessageRequest, GetMessagesQuery};
use crate::models::user::PartialUser;
use crate::services::{mention_service, permission_service, queries, thread, user};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake};

#[derive(FromRow)]
struct ChannelMetaRow {
    guild_id: String,
    #[sqlx(rename = "type")]
    channel_type: u32,
}

#[derive(FromRow)]
struct MessageMentionUserRow {
    message_id: String,
    id: String,
    username: String,
    discriminator: String,
    global_name: Option<String>,
    avatar: Option<String>,
    bot: bool,
    public_flags: u32,
}

#[derive(FromRow)]
struct EditTargetRow {
    author_id: String,
    #[sqlx(rename = "type")]
    kind: u32,
    flags: u32,
    mention_everyone: bool,
    edited_timestamp: Option<u64>,
    referenced_message_id: Option<String>,
    content: Option<String>,
}

fn validate_message_content(content: &str) -> Result<(), AppError> {
    if content.trim().is_empty() {
        return Err(AppError::empty_message());
    }
    if content.chars().count() > MAX_MESSAGE_LENGTH {
        return Err(AppError::validation(
            "content",
            ValidationCode::ContentExceedsMaxLength,
            format!("Text must not exceed {MAX_MESSAGE_LENGTH} characters."),
        ));
    }
    Ok(())
}

fn validate_nonce(raw: Option<String>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MAX_MESSAGE_NONCE_LENGTH || trimmed.chars().any(char::is_control) {
        return Err(AppError::validation(
            "nonce",
            ValidationCode::StringLengthInvalid,
            "Nonce must be between 1 and 32 characters.",
        ));
    }
    Ok(Some(trimmed))
}

fn parse_snowflake_opt(field: &str, value: Option<&str>) -> Result<Option<u64>, AppError> {
    let Some(s) = value else {
        return Ok(None);
    };
    validate_snowflake(field, s)?;
    s.parse::<u64>().map(Some).map_err(|_| {
        AppError::validation(
            field,
            ValidationCode::InvalidFormat,
            format!("Invalid snowflake for '{field}'."),
        )
    })
}

async fn get_channel_meta(state: &AppState, channel_id: &str) -> Result<ChannelMetaRow, AppError> {
    sqlx::query_as::<_, ChannelMetaRow>("SELECT guild_id, type FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_channel)
}

async fn fetch_message_member(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    preloaded_roles: Option<Vec<String>>,
) -> Option<MessageMember> {
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
    let roles = match preloaded_roles {
        Some(r) => r,
        None => queries::fetch_member_roles(&state.db, guild_id, user_id)
            .await
            .unwrap_or_default(),
    };

    Some(MessageMember::new(nick, roles, joined_at))
}

async fn fetch_message_row(state: &AppState, message_id: &str) -> Result<MessageRow, AppError> {
    sqlx::query_as::<_, MessageRow>(&format!("{} WHERE m.id = ?", MessageRow::SELECT_FROM))
        .bind(message_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_message)
}

pub async fn hydrate_messages(
    state: &AppState,
    rows: Vec<MessageRow>,
    viewer_id: &str,
    can_use_threads: bool,
) -> Result<Vec<Message>, AppError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let ref_ids: Vec<String> = rows
        .iter()
        .filter(|r| r.kind == MessageType::Reply as u32)
        .filter_map(|r| r.referenced_message_id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let ref_rows: Vec<MessageRow> = if ref_ids.is_empty() {
        Vec::new()
    } else {
        let mut qb =
            QueryBuilder::<MySql>::new(format!("{} WHERE m.id IN (", MessageRow::SELECT_FROM));
        let mut sep = qb.separated(", ");
        for id in &ref_ids {
            sep.push_bind(id);
        }
        qb.push(")");
        qb.build_query_as::<MessageRow>()
            .fetch_all(&state.db)
            .await?
    };

    let all_msg_ids: Vec<String> = rows
        .iter()
        .chain(ref_rows.iter())
        .map(|r| r.id.clone())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    let mut mentions_by_msg: HashMap<String, Vec<PartialUser>> = HashMap::new();
    let mut role_mentions_by_msg: HashMap<String, Vec<String>> = HashMap::new();

    if !all_msg_ids.is_empty() {
        let mut mqb = QueryBuilder::<MySql>::new(
            r#"
            SELECT
                mm.message_id, u.id, u.username, u.discriminator, u.global_name,
                u.avatar, u.bot, u.public_flags
            FROM message_mentions mm
            INNER JOIN users u ON u.id = mm.user_id
            WHERE mm.message_id IN (
            "#,
        );
        let mut sep = mqb.separated(", ");
        for mid in &all_msg_ids {
            sep.push_bind(mid);
        }
        mqb.push(") ORDER BY CAST(u.id AS UNSIGNED) ASC");

        let m_rows = mqb
            .build_query_as::<MessageMentionUserRow>()
            .fetch_all(&state.db)
            .await?;
        for r in m_rows {
            mentions_by_msg
                .entry(r.message_id)
                .or_default()
                .push(PartialUser {
                    id: r.id,
                    username: r.username,
                    discriminator: r.discriminator,
                    global_name: r.global_name,
                    avatar: r.avatar,
                    avatar_color: None,
                    bot: r.bot,
                    flags: r.public_flags,
                });
        }

        let mut rqb = QueryBuilder::<MySql>::new(
            "SELECT message_id, role_id FROM message_mention_roles WHERE message_id IN (",
        );
        let mut sep = rqb.separated(", ");
        for mid in &all_msg_ids {
            sep.push_bind(mid);
        }
        rqb.push(") ORDER BY CAST(role_id AS UNSIGNED) ASC");

        let r_rows: Vec<(String, String)> = rqb.build_query_as().fetch_all(&state.db).await?;
        for (mid, rid) in r_rows {
            role_mentions_by_msg.entry(mid).or_default().push(rid);
        }
    }

    let mut threads_by_msg = if can_use_threads {
        let thread_ids: Vec<String> = rows
            .iter()
            .chain(ref_rows.iter())
            .filter(|r| (r.flags & MessageFlags::HAS_THREAD.bits()) != 0)
            .map(|r| r.id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        if thread_ids.is_empty() {
            HashMap::new()
        } else {
            thread::fetch_threads_for_messages(state, &thread_ids, viewer_id).await?
        }
    } else {
        HashMap::new()
    };

    let has_thread_mask = !MessageFlags::HAS_THREAD.bits();

    let mut hydrated_refs: HashMap<String, Message> = HashMap::with_capacity(ref_rows.len());
    for row in ref_rows {
        let mut msg = Message::from(row);
        if let Some(users) = mentions_by_msg.get(&msg.id) {
            msg.mentions = users.clone();
        }
        if let Some(roles) = role_mentions_by_msg.get(&msg.id) {
            msg.mention_roles = roles.clone();
        }
        if can_use_threads {
            if (msg.flags & MessageFlags::HAS_THREAD.bits()) != 0 {
                msg.thread = threads_by_msg.get(&msg.id).cloned().map(Box::new);
            }
        } else {
            msg.flags &= has_thread_mask;
        }
        msg.referenced_message = None;
        hydrated_refs.insert(msg.id.clone(), msg);
    }

    let mut messages = Vec::with_capacity(rows.len());
    for row in rows {
        let mut msg = Message::from(row);
        if let Some(users) = mentions_by_msg.remove(&msg.id) {
            msg.mentions = users;
        }
        if let Some(roles) = role_mentions_by_msg.remove(&msg.id) {
            msg.mention_roles = roles;
        }
        if can_use_threads {
            if (msg.flags & MessageFlags::HAS_THREAD.bits()) != 0 {
                msg.thread = threads_by_msg.remove(&msg.id).map(Box::new);
            }
        } else {
            msg.flags &= has_thread_mask;
        }
        if msg.kind == MessageType::Reply as u32 {
            let ref_msg = msg.message_reference.as_ref().and_then(|mr| {
                mr.message_id.as_deref().and_then(|mid| {
                    hydrated_refs
                        .get(mid)
                        .filter(|rm| rm.channel_id == mr.channel_id)
                        .cloned()
                        .map(Box::new)
                })
            });
            msg.referenced_message = Some(ref_msg);
        }
        messages.push(msg);
    }

    Ok(messages)
}

pub async fn get_messages(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    query: GetMessagesQuery,
) -> Result<Vec<Message>, AppError> {
    let before_num = parse_snowflake_opt("before", query.before.as_deref())?;
    let after_num = parse_snowflake_opt("after", query.after.as_deref())?;
    let around_num = parse_snowflake_opt("around", query.around.as_deref())?;

    let meta = get_channel_meta(state, channel_id).await?;
    if !can_use_threads
        && (ChannelType::is_forum_like(meta.channel_type)
            || ChannelType::is_thread(meta.channel_type))
    {
        return Err(AppError::unknown_channel());
    }

    let perms = permission_service::require_channel_permission(
        state,
        &meta.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await?;
    if !ChannelType::supports_messages(meta.channel_type) {
        return Err(AppError::invalid_channel_type());
    }

    let read_perms = if ChannelType::is_thread(meta.channel_type) {
        thread::require_thread_message_access(state, channel_id, user_id)
            .await?
            .thread_perms
    } else {
        perms
    };

    if !read_perms.contains(PermissionFlags::READ_MESSAGE_HISTORY) {
        return Ok(vec![]);
    }

    let limit = query
        .limit
        .unwrap_or(DEFAULT_MESSAGES_PAGE_LIMIT)
        .clamp(1, MAX_MESSAGES_PAGE_LIMIT);

    let select = MessageRow::SELECT_FROM;
    let rows = if let Some(around) = around_num {
        let half_newer = limit / 2;
        let mut newer = if half_newer > 0 {
            let mut n = sqlx::query_as::<_, MessageRow>(&format!(
                "{select} WHERE m.channel_id = ? AND m.id_num > ? ORDER BY m.id_num ASC LIMIT ?"
            ))
            .bind(channel_id)
            .bind(around)
            .bind(half_newer)
            .fetch_all(&state.db)
            .await?;
            n.reverse();
            n
        } else {
            Vec::new()
        };

        let remaining = limit.saturating_sub(newer.len() as u32).max(1);

        let older = sqlx::query_as::<_, MessageRow>(&format!(
            "{select} WHERE m.channel_id = ? AND m.id_num <= ? ORDER BY m.id_num DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(around)
        .bind(remaining)
        .fetch_all(&state.db)
        .await?;

        newer.extend(older);
        newer
    } else if let Some(after) = after_num {
        let mut rows = sqlx::query_as::<_, MessageRow>(&format!(
            "{select} WHERE m.channel_id = ? AND m.id_num > ? ORDER BY m.id_num ASC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(after)
        .bind(limit)
        .fetch_all(&state.db)
        .await?;
        rows.reverse();
        rows
    } else if let Some(before) = before_num {
        sqlx::query_as::<_, MessageRow>(&format!(
            "{select} WHERE m.channel_id = ? AND m.id_num < ? ORDER BY m.id_num DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, MessageRow>(&format!(
            "{select} WHERE m.channel_id = ? ORDER BY m.id_num DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    };

    hydrate_messages(state, rows, user_id, can_use_threads).await
}

pub async fn create_message(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    is_bot: bool,
    payload: CreateMessageRequest,
) -> Result<Message, AppError> {
    let content = payload.content.unwrap_or_default();
    validate_message_content(&content)?;
    let nonce = validate_nonce(payload.nonce)?;

    let allowed_create_flags =
        MessageFlags::SUPPRESS_EMBEDS.bits() | MessageFlags::SUPPRESS_NOTIFICATIONS.bits();
    let msg_flags = payload.flags.unwrap_or(0) & allowed_create_flags;
    let suppress_notifications = (msg_flags & MessageFlags::SUPPRESS_NOTIFICATIONS.bits()) != 0;

    let meta = get_channel_meta(state, channel_id).await?;
    if !can_use_threads
        && (ChannelType::is_forum_like(meta.channel_type)
            || ChannelType::is_thread(meta.channel_type))
    {
        return Err(AppError::unknown_channel());
    }
    if !ChannelType::supports_messages(meta.channel_type) {
        return Err(AppError::cannot_send_in_non_text_channel());
    }

    let is_thread = ChannelType::is_thread(meta.channel_type);
    let (perm_ctx, member_roles, sender_perms, thread_ctx) = if is_thread {
        let ctx = thread::require_thread_message_access(state, channel_id, user_id).await?;
        if !ctx
            .thread_perms
            .contains(PermissionFlags::SEND_MESSAGES_IN_THREADS)
        {
            return Err(AppError::missing_permissions());
        }
        if ctx.row.locked && !ctx.is_moderator {
            return Err(AppError::thread_locked());
        }
        let perms = ctx.thread_perms;
        (None, None, perms, Some(ctx))
    } else {
        let (ctx, roles, perms) = permission_service::require_channel_permission_with_ctx(
            state,
            &meta.guild_id,
            channel_id,
            user_id,
            PermissionFlags::VIEW_CHANNEL | PermissionFlags::SEND_MESSAGES,
        )
        .await?;
        (ctx, roles, perms, None)
    };

    let (message_reference, referenced_message, reply_author_id) = match payload.message_reference {
        None => (None, None, None),
        Some(ref_input) => {
            let ref_kind = ref_input
                .kind
                .unwrap_or(MessageReferenceType::Default as u8);
            if ref_kind != MessageReferenceType::Default as u8 {
                return Err(AppError::validation(
                    "message_reference.type",
                    ValidationCode::InvalidFormat,
                    "Message forwarding is not supported.",
                ));
            }
            validate_snowflake("message_reference.message_id", &ref_input.message_id)?;
            if let Some(ref cid) = ref_input.channel_id {
                validate_snowflake("message_reference.channel_id", cid)?;
                if cid != channel_id {
                    return Err(AppError::validation(
                        "message_reference.channel_id",
                        ValidationCode::InvalidFormat,
                        "Cannot reply to a message in another channel.",
                    ));
                }
            }
            if let Some(ref gid) = ref_input.guild_id {
                validate_snowflake("message_reference.guild_id", gid)?;
                if gid != &meta.guild_id {
                    return Err(AppError::validation(
                        "message_reference.guild_id",
                        ValidationCode::InvalidFormat,
                        "Cannot reply to a message in another community.",
                    ));
                }
            }

            let target_row = sqlx::query_as::<_, MessageRow>(&format!(
                "{} WHERE m.id = ? AND m.channel_id = ?",
                MessageRow::SELECT_FROM
            ))
            .bind(&ref_input.message_id)
            .bind(channel_id)
            .fetch_optional(&state.db)
            .await?;

            let (hydrated_target, target_author_id) = match target_row {
                Some(row) => {
                    if row.kind != MessageType::Default as u32
                        && row.kind != MessageType::Reply as u32
                    {
                        return Err(AppError::validation(
                            "message_reference",
                            ValidationCode::InvalidFormat,
                            "Cannot reply to a system message.",
                        ));
                    }
                    let mut list =
                        hydrate_messages(state, vec![row], user_id, can_use_threads).await?;
                    let target = list.pop().map(|mut m| {
                        m.referenced_message = None;
                        m
                    });
                    let author_id = target.as_ref().map(|m| m.author.id.clone());
                    (target.map(Box::new), author_id)
                }
                None => {
                    if ref_input.fail_if_not_exists.unwrap_or(true) {
                        return Err(AppError::unknown_message());
                    }
                    (None, None)
                }
            };

            let reference = MessageReference {
                channel_id: channel_id.to_string(),
                message_id: Some(ref_input.message_id),
                guild_id: Some(meta.guild_id.clone()),
                kind: MessageReferenceType::Default as u8,
            };

            (Some(reference), Some(hydrated_target), target_author_id)
        }
    };

    let msg_kind = if message_reference.is_some() {
        MessageType::Reply as u32
    } else {
        MessageType::Default as u32
    };

    let resolved = mention_service::resolve_mentions(
        state,
        &meta.guild_id,
        user_id,
        sender_perms,
        &content,
        payload.allowed_mentions.as_ref(),
        reply_author_id.as_deref(),
    )
    .await?;

    let message_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let timestamp = Utc::now().timestamp_millis() as u64;

    let mut tx = state.db.begin().await?;

    sqlx::query(
        r#"
        INSERT INTO messages (
            id, channel_id, author_id, content, timestamp, type, flags, mention_everyone,
            referenced_message_id, referenced_channel_id, referenced_guild_id, reference_type
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&message_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(&content)
    .bind(timestamp)
    .bind(msg_kind)
    .bind(msg_flags)
    .bind(resolved.mention_everyone)
    .bind(
        message_reference
            .as_ref()
            .and_then(|r| r.message_id.as_deref()),
    )
    .bind(message_reference.as_ref().map(|r| r.channel_id.as_str()))
    .bind(
        message_reference
            .as_ref()
            .and_then(|r| r.guild_id.as_deref()),
    )
    .bind(message_reference.as_ref().map(|r| r.kind))
    .execute(&mut *tx)
    .await?;

    mention_service::persist_mentions_tx(
        &mut tx,
        &message_id,
        &resolved.mentions,
        &resolved.mention_roles,
        false,
    )
    .await?;

    let thread_post_effects = if let Some(ref ctx) = thread_ctx {
        Some(
            thread::apply_thread_message_created(
                &mut tx,
                ctx,
                user_id,
                &message_id,
                timestamp,
                is_bot,
            )
            .await?,
        )
    } else {
        sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
            .bind(&message_id)
            .bind(channel_id)
            .execute(&mut *tx)
            .await?;
        None
    };

    if !suppress_notifications {
        let parent_id = thread_ctx.as_ref().map(|ctx| ctx.row.parent_id.as_str());
        mention_service::increment_mention_counts_tx(
            &mut tx,
            &meta.guild_id,
            channel_id,
            meta.channel_type,
            parent_id,
            user_id,
            &resolved,
        )
        .await?;
    }

    if !is_bot {
        crate::services::read_state_service::mark_channel_read_on_send_tx(
            &mut tx,
            user_id,
            channel_id,
            &message_id,
        )
        .await?;
    }

    tx.commit().await?;

    let author = match state.hub.get_cached_partial_user(user_id) {
        Some(u) => u,
        None => user::get_partial_user(state, user_id).await?,
    };

    let msg = Message {
        id: message_id,
        channel_id: channel_id.to_string(),
        author,
        kind: msg_kind,
        flags: msg_flags,
        content,
        timestamp,
        edited_timestamp: None,
        pinned: false,
        mention_everyone: resolved.mention_everyone,
        tts: false,
        mentions: resolved.mentions,
        mention_roles: resolved.mention_roles,
        embeds: vec![],
        attachments: vec![],
        stickers: vec![],
        message_reference,
        nonce,
        referenced_message,
        thread: None,
    };

    let member = fetch_message_member(state, &meta.guild_id, user_id, member_roles).await;
    let event = GatewayMessageEvent {
        message: msg.clone(),
        channel_type: meta.channel_type,
        guild_id: Some(meta.guild_id.clone()),
        member,
    };

    let dispatch_channel_id = thread_ctx
        .as_ref()
        .map_or(channel_id, |ctx| ctx.row.parent_id.as_str());

    permission_service::dispatch_channel_with_ctx(
        state,
        &meta.guild_id,
        dispatch_channel_id,
        perm_ctx.as_ref(),
        events::MESSAGE_CREATE,
        &event,
    )
    .await;

    if !is_bot {
        crate::services::read_state_service::dispatch_message_ack(
            state,
            user_id,
            meta.channel_type,
            channel_id,
            &msg.id,
            0,
            false,
        )
        .await;
    }

    if let (Some(ctx), Some((unarchived, added_info))) = (thread_ctx, thread_post_effects) {
        thread::dispatch_thread_after_message(state, &ctx, user_id, unarchived, added_info).await;
    }

    Ok(msg)
}

pub async fn edit_message(
    state: &AppState,
    channel_id: &str,
    message_id: &str,
    user_id: &str,
    can_use_threads: bool,
    payload: EditMessageRequest,
) -> Result<Message, AppError> {
    let meta = get_channel_meta(state, channel_id).await?;
    if !can_use_threads
        && (ChannelType::is_forum_like(meta.channel_type)
            || ChannelType::is_thread(meta.channel_type))
    {
        return Err(AppError::unknown_channel());
    }

    let is_thread = ChannelType::is_thread(meta.channel_type);
    let (perm_ctx, member_roles, perms, thread_ctx) = if is_thread {
        let ctx = thread::require_thread_message_access(state, channel_id, user_id).await?;
        if ctx.row.archived {
            return Err(AppError::thread_archived());
        }
        if ctx.row.locked && !ctx.is_moderator {
            return Err(AppError::thread_locked());
        }
        let perms = ctx.thread_perms;
        (None, None, perms, Some(ctx))
    } else {
        let (ctx, roles, p) = permission_service::require_channel_permission_with_ctx(
            state,
            &meta.guild_id,
            channel_id,
            user_id,
            PermissionFlags::VIEW_CHANNEL,
        )
        .await?;
        (ctx, roles, p, None)
    };

    let target = sqlx::query_as::<_, EditTargetRow>(
        "SELECT author_id, type, flags, mention_everyone, edited_timestamp, referenced_message_id, content FROM messages WHERE id = ? AND channel_id = ?",
    )
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_message)?;

    if target.kind != MessageType::Default as u32 && target.kind != MessageType::Reply as u32 {
        return Err(AppError::cannot_modify_system_message(
            "Cannot edit a system message.",
        ));
    }

    if target.author_id != user_id {
        if payload.content.is_some() || payload.allowed_mentions.is_some() {
            return Err(AppError::cannot_edit_other_message());
        }
        if payload.flags.is_none() || !perms.contains(PermissionFlags::MANAGE_MESSAGES) {
            return Err(AppError::cannot_edit_other_message());
        }
    }

    let content_changed = payload.content.is_some();
    let next_content = match payload.content {
        Some(c) => {
            validate_message_content(&c)?;
            c
        }
        None => target.content.unwrap_or_default(),
    };

    let next_flags = match payload.flags {
        Some(req_flags) => {
            let mask = MessageFlags::SUPPRESS_EMBEDS.bits();
            (target.flags & !mask) | (req_flags & mask)
        }
        None => target.flags,
    };

    let next_edited_timestamp = if content_changed {
        Some(Utc::now().timestamp_millis() as u64)
    } else {
        target.edited_timestamp
    };

    let reparse_mentions = content_changed || payload.allowed_mentions.is_some();
    let resolved_mentions = if reparse_mentions {
        let reply_author_id: Option<String> = if target.kind == MessageType::Reply as u32
            && let Some(ref ref_mid) = target.referenced_message_id
        {
            sqlx::query_scalar("SELECT author_id FROM messages WHERE id = ? AND channel_id = ?")
                .bind(ref_mid)
                .bind(channel_id)
                .fetch_optional(&state.db)
                .await?
        } else {
            None
        };

        Some(
            mention_service::resolve_mentions(
                state,
                &meta.guild_id,
                &target.author_id,
                perms,
                &next_content,
                payload.allowed_mentions.as_ref(),
                reply_author_id.as_deref(),
            )
            .await?,
        )
    } else {
        None
    };

    let next_mention_everyone = resolved_mentions
        .as_ref()
        .map_or(target.mention_everyone, |r| r.mention_everyone);

    let mut tx = state.db.begin().await?;

    sqlx::query(
        "UPDATE messages SET content = ?, flags = ?, mention_everyone = ?, edited_timestamp = ? WHERE id = ?",
    )
    .bind(&next_content)
    .bind(next_flags)
    .bind(next_mention_everyone)
    .bind(next_edited_timestamp)
    .bind(message_id)
    .execute(&mut *tx)
    .await?;

    if let Some(ref resolved) = resolved_mentions {
        mention_service::persist_mentions_tx(
            &mut tx,
            message_id,
            &resolved.mentions,
            &resolved.mention_roles,
            true,
        )
        .await?;
    }

    tx.commit().await?;

    let updated_row = fetch_message_row(state, message_id).await?;
    let updated = hydrate_messages(state, vec![updated_row], user_id, can_use_threads)
        .await?
        .into_iter()
        .next()
        .ok_or_else(AppError::unknown_message)?;

    let author_roles = if target.author_id == user_id {
        member_roles
    } else {
        None
    };
    let member = fetch_message_member(state, &meta.guild_id, &target.author_id, author_roles).await;
    let event = GatewayMessageEvent {
        message: updated.clone(),
        channel_type: meta.channel_type,
        guild_id: Some(meta.guild_id.clone()),
        member,
    };

    let dispatch_channel_id = thread_ctx
        .as_ref()
        .map_or(channel_id, |ctx| ctx.row.parent_id.as_str());
    permission_service::dispatch_channel_with_ctx(
        state,
        &meta.guild_id,
        dispatch_channel_id,
        perm_ctx.as_ref(),
        events::MESSAGE_UPDATE,
        &event,
    )
    .await;

    Ok(updated)
}

pub async fn delete_message(
    state: &AppState,
    channel_id: &str,
    message_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<(), AppError> {
    let meta = get_channel_meta(state, channel_id).await?;
    if !can_use_threads
        && (ChannelType::is_forum_like(meta.channel_type)
            || ChannelType::is_thread(meta.channel_type))
    {
        return Err(AppError::unknown_channel());
    }
    let is_thread = ChannelType::is_thread(meta.channel_type);
    let (perm_ctx, perms, thread_ctx) = if is_thread {
        let ctx = thread::require_thread_message_access(state, channel_id, user_id).await?;
        (None, ctx.thread_perms, Some(ctx))
    } else {
        let (ctx, _, p) = permission_service::require_channel_permission_with_ctx(
            state,
            &meta.guild_id,
            channel_id,
            user_id,
            PermissionFlags::VIEW_CHANNEL,
        )
        .await?;
        (ctx, p, None)
    };

    let (author_id, kind): (String, u32) =
        sqlx::query_as("SELECT author_id, type FROM messages WHERE id = ? AND channel_id = ?")
            .bind(message_id)
            .bind(channel_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(AppError::unknown_message)?;

    let deletable = kind == MessageType::Default as u32
        || kind == MessageType::Reply as u32
        || kind == MessageType::ChannelPinnedMessage as u32
        || kind == MessageType::UserJoin as u32
        || kind == MessageType::ChannelFollowAdd as u32
        || kind == MessageType::ThreadCreated as u32;
    if !deletable {
        return Err(AppError::cannot_modify_system_message(
            "Cannot delete this message type.",
        ));
    }

    if author_id != user_id && !perms.contains(PermissionFlags::MANAGE_MESSAGES) {
        return Err(AppError::missing_permissions());
    }

    let mut tx = state.db.begin().await?;

    sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(message_id)
        .execute(&mut *tx)
        .await?;

    if let Some(ref ctx) = thread_ctx {
        thread::apply_thread_message_deleted(&mut tx, ctx, message_id).await?;
    }

    sqlx::query(
        r#"
        UPDATE channels
        SET last_message_id = (
            SELECT m.id FROM messages m
            WHERE m.channel_id = ?
            ORDER BY m.id_num DESC
            LIMIT 1
        )
        WHERE id = ? AND last_message_id = ?
        "#,
    )
    .bind(channel_id)
    .bind(channel_id)
    .bind(message_id)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    let dispatch_channel_id = thread_ctx
        .as_ref()
        .map_or(channel_id, |ctx| ctx.row.parent_id.as_str());
    permission_service::dispatch_channel_with_ctx(
        state,
        &meta.guild_id,
        dispatch_channel_id,
        perm_ctx.as_ref(),
        events::MESSAGE_DELETE,
        &MessageDeleteEvent {
            id: message_id,
            channel_id,
            guild_id: &meta.guild_id,
        },
    )
    .await;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_message_content_and_nonce() {
        assert!(validate_message_content("   ").is_err());
        assert!(validate_message_content("hello").is_ok());
        assert!(validate_message_content(&"a".repeat(MAX_MESSAGE_LENGTH + 1)).is_err());

        assert_eq!(validate_nonce(None).unwrap(), None);
        assert_eq!(validate_nonce(Some("   ".to_string())).unwrap(), None);
        assert_eq!(
            validate_nonce(Some(" 12345 ".to_string())).unwrap(),
            Some("12345".to_string())
        );
        assert!(validate_nonce(Some("x".repeat(MAX_MESSAGE_NONCE_LENGTH + 1))).is_err());
        assert!(validate_nonce(Some("bad\nnonce".to_string())).is_err());
    }

    #[test]
    fn parses_optional_snowflakes() {
        assert_eq!(parse_snowflake_opt("before", None).unwrap(), None);
        assert_eq!(
            parse_snowflake_opt("before", Some("123456789012345678")).unwrap(),
            Some(123456789012345678)
        );
        assert!(parse_snowflake_opt("before", Some("abc")).is_err());
    }
}
