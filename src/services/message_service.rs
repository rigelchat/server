use chrono::Utc;
use serde_json::json;

use crate::AppState;
use crate::constants::gateway::events;
use crate::error::AppError;
use crate::models::message::{Message, MessageDbRow, MessageType};
use crate::services::guild_service;
use crate::utils::snowflake::DISCORD_SNOWFLAKE;

async fn get_channel_guild_id(state: &AppState, channel_id: &str) -> Result<String, AppError> {
    sqlx::query_scalar::<_, String>("SELECT guild_id FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Channel not found".to_string()))
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
    .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;

    Ok(Message::from(row))
}

pub async fn get_messages(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> Result<Vec<Message>, AppError> {
    let guild_id = get_channel_guild_id(state, channel_id).await?;
    guild_service::require_member(state, &guild_id, user_id).await?;

    let rows = sqlx::query_as::<_, MessageDbRow>(
        r#"
        SELECT
            m.id, m.channel_id, m.content, m.timestamp, m.edited_timestamp,
            m.type AS kind, m.flags,
            u.id AS author_id, u.username, u.discriminator, u.global_name, u.avatar,
            u.bot, u.public_flags
        FROM messages m
        INNER JOIN users u ON u.id = m.author_id
        WHERE m.channel_id = ?
        ORDER BY m.timestamp DESC, m.id DESC
        LIMIT 50
        "#,
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await?;

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
        return Err(AppError::BadRequest("Message content cannot be empty".to_string()));
    }

    let guild_id = get_channel_guild_id(state, channel_id).await?;
    guild_service::require_member(state, &guild_id, user_id).await?;

    let message_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let timestamp = Utc::now().timestamp_millis() as u64;

    sqlx::query(
        "INSERT INTO messages (id, channel_id, author_id, content, timestamp, type) VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&message_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(&content)
    .bind(timestamp)
    .bind(MessageType::Default as u32)
    .execute(&state.db)
    .await?;

    let mut msg = fetch_message(state, &message_id).await?;
    msg.nonce = nonce;

    state
        .dispatch_guild(events::MESSAGE_CREATE, &msg, &guild_id)
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
        return Err(AppError::BadRequest("Message content cannot be empty".to_string()));
    }

    let guild_id = get_channel_guild_id(state, channel_id).await?;
    guild_service::require_member(state, &guild_id, user_id).await?;

    let author_id = sqlx::query_scalar::<_, String>(
        "SELECT author_id FROM messages WHERE id = ? AND channel_id = ?",
    )
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;

    if author_id != user_id {
        return Err(AppError::Forbidden(
            "Cannot edit a message authored by another user".to_string(),
        ));
    }

    let edited_timestamp = Utc::now().timestamp_millis() as u64;

    sqlx::query("UPDATE messages SET content = ?, edited_timestamp = ? WHERE id = ?")
        .bind(&content)
        .bind(edited_timestamp)
        .bind(message_id)
        .execute(&state.db)
        .await?;

    let updated = fetch_message(state, message_id).await?;
    state
        .dispatch_guild(events::MESSAGE_UPDATE, &updated, &guild_id)
        .await;

    Ok(updated)
}

pub async fn delete_message(
    state: &AppState,
    channel_id: &str,
    message_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let guild_id = get_channel_guild_id(state, channel_id).await?;
    guild_service::require_member(state, &guild_id, user_id).await?;

    let author_id = sqlx::query_scalar::<_, String>(
        "SELECT author_id FROM messages WHERE id = ? AND channel_id = ?",
    )
    .bind(message_id)
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Message not found".to_string()))?;

    if author_id != user_id {
        return Err(AppError::Forbidden("Missing permissions".to_string()));
    }

    sqlx::query("DELETE FROM messages WHERE id = ?")
        .bind(message_id)
        .execute(&state.db)
        .await?;

    state
        .dispatch_guild(
            events::MESSAGE_DELETE,
            &json!({
                "id": message_id,
                "channel_id": channel_id,
                "guild_id": guild_id,
            }),
            &guild_id,
        )
        .await;

    Ok(())
}