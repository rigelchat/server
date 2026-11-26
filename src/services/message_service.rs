use chrono::Utc;
use sqlx::FromRow;

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
use crate::models::message::{Message, MessageDbRow};
use crate::models::rest::messages::GetMessagesQuery;
use crate::services::{permission_service, queries, thread, user};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake};

#[derive(FromRow)]
struct ChannelMetaRow {
    guild_id: String,
    #[sqlx(rename = "type")]
    channel_type: u32,
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

async fn fetch_message(state: &AppState, message_id: &str) -> Result<Message, AppError> {
    let row = sqlx::query_as::<_, MessageDbRow>(&format!("{SELECT_MSG_COLS} WHERE m.id = ?"))
        .bind(message_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_message)?;

    Ok(Message::from(row))
}

const SELECT_MSG_COLS: &str = r#"
    SELECT
        m.id, m.channel_id, m.content, m.timestamp, m.edited_timestamp,
        m.type AS kind, m.flags,
        u.id AS author_id, u.username, u.discriminator, u.global_name, u.avatar,
        u.bot, u.public_flags
    FROM messages m
    INNER JOIN users u ON u.id = m.author_id
"#;

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

    let rows = if let Some(around) = around_num {
        let half_newer = limit / 2;
        let mut newer = if half_newer > 0 {
            let mut n = sqlx::query_as::<_, MessageDbRow>(&format!(
                "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND m.id_num > ? ORDER BY m.id_num ASC LIMIT ?"
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

        let older = sqlx::query_as::<_, MessageDbRow>(&format!(
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND m.id_num <= ? ORDER BY m.id_num DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(around)
        .bind(remaining)
        .fetch_all(&state.db)
        .await?;

        newer.extend(older);
        newer
    } else if let Some(after) = after_num {
        let mut rows = sqlx::query_as::<_, MessageDbRow>(&format!(
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND m.id_num > ? ORDER BY m.id_num ASC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(after)
        .bind(limit)
        .fetch_all(&state.db)
        .await?;
        rows.reverse();
        rows
    } else if let Some(before) = before_num {
        sqlx::query_as::<_, MessageDbRow>(&format!(
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND m.id_num < ? ORDER BY m.id_num DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, MessageDbRow>(&format!(
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? ORDER BY m.id_num DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    };

    let mut messages: Vec<Message> = rows.into_iter().map(Message::from).collect();
    if can_use_threads {
        let thread_ids: Vec<String> = messages
            .iter()
            .filter(|m| (m.flags & MessageFlags::HAS_THREAD.bits()) != 0)
            .map(|m| m.id.clone())
            .collect();
        if !thread_ids.is_empty() {
            let mut by_id = thread::fetch_threads_for_messages(state, &thread_ids, user_id).await?;
            for m in &mut messages {
                if (m.flags & MessageFlags::HAS_THREAD.bits()) != 0 {
                    m.thread = by_id.remove(&m.id).map(Box::new);
                }
            }
        }
    } else {
        let mask = !MessageFlags::HAS_THREAD.bits();
        for m in &mut messages {
            m.flags &= mask;
        }
    }
    Ok(messages)
}

pub async fn create_message(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    is_bot: bool,
    content: String,
    nonce: Option<String>,
) -> Result<Message, AppError> {
    validate_message_content(&content)?;
    let nonce = validate_nonce(nonce)?;

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
    let (perm_ctx, member_roles, thread_ctx) = if is_thread {
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
        (None, None, Some(ctx))
    } else {
        let (ctx, roles, _) = permission_service::require_channel_permission_with_ctx(
            state,
            &meta.guild_id,
            channel_id,
            user_id,
            PermissionFlags::VIEW_CHANNEL | PermissionFlags::SEND_MESSAGES,
        )
        .await?;
        (ctx, roles, None)
    };

    let message_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let timestamp = Utc::now().timestamp_millis() as u64;

    let mut tx = state.db.begin().await?;

    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, timestamp, type) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(&content)
    .bind(timestamp)
    .bind(MessageType::Default as u32)
    .execute(&mut *tx)
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
        kind: MessageType::Default as u32,
        flags: 0,
        content,
        timestamp,
        edited_timestamp: None,
        pinned: false,
        mention_everyone: false,
        tts: false,
        mentions: vec![],
        mention_roles: vec![],
        embeds: vec![],
        attachments: vec![],
        stickers: vec![],
        nonce,
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
    content: String,
) -> Result<Message, AppError> {
    validate_message_content(&content)?;

    let meta = get_channel_meta(state, channel_id).await?;
    if !can_use_threads
        && (ChannelType::is_forum_like(meta.channel_type)
            || ChannelType::is_thread(meta.channel_type))
    {
        return Err(AppError::unknown_channel());
    }
    let is_thread = ChannelType::is_thread(meta.channel_type);
    let (perm_ctx, member_roles, thread_ctx) = if is_thread {
        let ctx = thread::require_thread_message_access(state, channel_id, user_id).await?;
        if ctx.row.archived {
            return Err(AppError::thread_archived());
        }
        if ctx.row.locked && !ctx.is_moderator {
            return Err(AppError::thread_locked());
        }
        (None, None, Some(ctx))
    } else {
        let (ctx, roles, _) = permission_service::require_channel_permission_with_ctx(
            state,
            &meta.guild_id,
            channel_id,
            user_id,
            PermissionFlags::VIEW_CHANNEL,
        )
        .await?;
        (ctx, roles, None)
    };

    let author_id = sqlx::query_scalar::<_, String>(
        "SELECT author_id FROM messages WHERE id = ? AND channel_id = ?",
    )
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_message)?;

    if author_id != user_id {
        return Err(AppError::cannot_edit_other_message());
    }

    let edited_timestamp = Utc::now().timestamp_millis() as u64;

    sqlx::query("UPDATE messages SET content = ?, edited_timestamp = ? WHERE id = ?")
        .bind(&content)
        .bind(edited_timestamp)
        .bind(message_id)
        .execute(&state.db)
        .await?;

    let mut updated = fetch_message(state, message_id).await?;
    if can_use_threads && (updated.flags & MessageFlags::HAS_THREAD.bits()) != 0 {
        let mut map =
            thread::fetch_threads_for_messages(state, std::slice::from_ref(&updated.id), user_id)
                .await?;
        updated.thread = map.remove(&updated.id).map(Box::new);
    } else if !can_use_threads {
        updated.flags &= !MessageFlags::HAS_THREAD.bits();
    }

    let member = fetch_message_member(state, &meta.guild_id, user_id, member_roles).await;
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

    let author_id = sqlx::query_scalar::<_, String>(
        "SELECT author_id FROM messages WHERE id = ? AND channel_id = ?",
    )
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_message)?;

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
