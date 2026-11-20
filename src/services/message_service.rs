use chrono::Utc;
use serde_json::json;
use sqlx::FromRow;

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::{
    DEFAULT_MESSAGES_PAGE_LIMIT, MAX_MESSAGE_LENGTH, MAX_MESSAGES_PAGE_LIMIT,
};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::ChannelType;
use crate::models::message::{
    GatewayMessageEvent, GetMessagesQuery, Message, MessageDbRow, MessageMember, MessageType,
};
use crate::services::{member_service, permission_service};
use crate::utils::snowflake::DISCORD_SNOWFLAKE;

#[derive(FromRow)]
struct ChannelMetaRow {
    guild_id: String,
    #[sqlx(rename = "type")]
    channel_type: u32,
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
    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(guild_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    Some(MessageMember {
        nick,
        avatar: None,
        banner: None,
        accent_color: None,
        roles,
        joined_at,
        mute: false,
        deaf: false,
        communication_disabled_until: None,
    })
}

async fn fetch_message(state: &AppState, message_id: &str) -> Result<Message, AppError> {
    let row = sqlx::query_as::<_, MessageDbRow>(
        r#"
        SELECT
            m.id, m.channel_id, m.content, m.timestamp, m.edited_timestamp,
            m.type AS kind, m.flags,
            u.id AS author_id, u.username, u.discriminator, u.global_name, u.avatar,
            u.bot, u.public_flags
        FROM messages m
        INNER JOIN users u ON u.id = m.author_id
        WHERE m.id = ?
        "#,
    )
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
    query: GetMessagesQuery,
) -> Result<Vec<Message>, AppError> {
    let meta = get_channel_meta(state, channel_id).await?;
    member_service::require_member(state, &meta.guild_id, user_id).await?;
    if meta.channel_type == ChannelType::GuildCategory as u32 {
        return Err(AppError::invalid_channel_type());
    }

    let perms = permission_service::require_channel_permission(
        state,
        &meta.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await?;

    if !perms.contains(PermissionFlags::READ_MESSAGE_HISTORY) {
        return Ok(vec![]);
    }

    let limit = query
        .limit
        .unwrap_or(DEFAULT_MESSAGES_PAGE_LIMIT)
        .clamp(1, MAX_MESSAGES_PAGE_LIMIT);

    let rows = if let Some(around) = &query.around {
        let half_newer = limit / 2;
        let mut newer = if half_newer > 0 {
            let mut n = sqlx::query_as::<_, MessageDbRow>(&format!(
                "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND CAST(m.id AS UNSIGNED) > CAST(? AS UNSIGNED) ORDER BY CAST(m.id AS UNSIGNED) ASC LIMIT ?"
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
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND CAST(m.id AS UNSIGNED) <= CAST(? AS UNSIGNED) ORDER BY CAST(m.id AS UNSIGNED) DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(around)
        .bind(remaining)
        .fetch_all(&state.db)
        .await?;

        newer.extend(older);
        newer
    } else if let Some(after) = &query.after {
        let mut rows = sqlx::query_as::<_, MessageDbRow>(&format!(
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND CAST(m.id AS UNSIGNED) > CAST(? AS UNSIGNED) ORDER BY CAST(m.id AS UNSIGNED) ASC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(after)
        .bind(limit)
        .fetch_all(&state.db)
        .await?;
        rows.reverse();
        rows
    } else if let Some(before) = &query.before {
        sqlx::query_as::<_, MessageDbRow>(&format!(
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? AND CAST(m.id AS UNSIGNED) < CAST(? AS UNSIGNED) ORDER BY CAST(m.id AS UNSIGNED) DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, MessageDbRow>(&format!(
            "{SELECT_MSG_COLS} WHERE m.channel_id = ? ORDER BY CAST(m.id AS UNSIGNED) DESC LIMIT ?"
        ))
        .bind(channel_id)
        .bind(limit)
        .fetch_all(&state.db)
        .await?
    };

    Ok(rows.into_iter().map(Message::from).collect())
}

pub async fn create_message(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    content: String,
    nonce: Option<String>,
) -> Result<Message, AppError> {
    if content.trim().is_empty() {
        return Err(AppError::empty_message());
    }
    let max_len = MAX_MESSAGE_LENGTH;
    if content.chars().count() > max_len {
        return Err(AppError::validation(
            "content",
            ValidationCode::ContentExceedsMaxLength,
            format!("Text must not exceed {max_len} characters."),
        ));
    }

    let meta = get_channel_meta(state, channel_id).await?;
    member_service::require_member(state, &meta.guild_id, user_id).await?;
    if meta.channel_type == ChannelType::GuildCategory as u32 {
        return Err(AppError::cannot_send_in_non_text_channel());
    }

    permission_service::require_channel_permission(
        state,
        &meta.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::SEND_MESSAGES,
    )
    .await?;

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

    sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
        .bind(&message_id)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let mut msg = fetch_message(state, &message_id).await?;
    msg.nonce = nonce;

    let member = fetch_message_member(state, &meta.guild_id, user_id).await;
    let event = GatewayMessageEvent {
        message: msg.clone(),
        channel_type: meta.channel_type,
        guild_id: Some(meta.guild_id.clone()),
        member,
    };

    permission_service::dispatch_channel(
        state,
        &meta.guild_id,
        channel_id,
        events::MESSAGE_CREATE,
        &event,
    )
    .await;

    Ok(msg)
}

pub async fn edit_message(
    state: &AppState,
    channel_id: &str,
    message_id: &str,
    user_id: &str,
    content: String,
) -> Result<Message, AppError> {
    if content.trim().is_empty() {
        return Err(AppError::empty_message());
    }
    let max_len = MAX_MESSAGE_LENGTH;
    if content.chars().count() > max_len {
        return Err(AppError::validation(
            "content",
            ValidationCode::ContentExceedsMaxLength,
            format!("Text must not exceed {max_len} characters."),
        ));
    }

    let meta = get_channel_meta(state, channel_id).await?;
    member_service::require_member(state, &meta.guild_id, user_id).await?;
    permission_service::require_channel_permission(
        state,
        &meta.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await?;

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

    let updated = fetch_message(state, message_id).await?;
    let member = fetch_message_member(state, &meta.guild_id, user_id).await;
    let event = GatewayMessageEvent {
        message: updated.clone(),
        channel_type: meta.channel_type,
        guild_id: Some(meta.guild_id.clone()),
        member,
    };

    permission_service::dispatch_channel(
        state,
        &meta.guild_id,
        channel_id,
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
) -> Result<(), AppError> {
    let meta = get_channel_meta(state, channel_id).await?;
    member_service::require_member(state, &meta.guild_id, user_id).await?;
    let perms = permission_service::require_channel_permission(
        state,
        &meta.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await?;

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

    sqlx::query(
        r#"
        UPDATE channels
        SET last_message_id = (
            SELECT m.id FROM messages m
            WHERE m.channel_id = ?
            ORDER BY CAST(m.id AS UNSIGNED) DESC
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

    permission_service::dispatch_channel(
        state,
        &meta.guild_id,
        channel_id,
        events::MESSAGE_DELETE,
        &json!({
            "id": message_id,
            "channel_id": channel_id,
            "guild_id": meta.guild_id,
        }),
    )
    .await;

    Ok(())
}