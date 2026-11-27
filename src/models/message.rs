use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::constants::message::MessageType;
use crate::models::serde_helpers::{serialize_millis_iso, serialize_millis_iso_opt};
use crate::models::thread::ThreadResponse;
use crate::models::user::PartialUser;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "u8")]
pub enum MessageReferenceType {
    Default = 0,
    Forward = 1,
}

impl From<MessageReferenceType> for u8 {
    fn from(value: MessageReferenceType) -> Self {
        value as u8
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageReference {
    pub channel_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: u8,
}

#[derive(Debug, Clone, FromRow)]
pub struct MessageRow {
    pub id: String,
    pub channel_id: String,
    pub content: Option<String>,
    pub timestamp: u64,
    pub edited_timestamp: Option<u64>,
    pub kind: u32,
    pub flags: u32,
    pub mention_everyone: bool,
    pub referenced_message_id: Option<String>,
    pub referenced_channel_id: Option<String>,
    pub referenced_guild_id: Option<String>,
    pub reference_type: Option<u8>,
    pub author_id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub bot: bool,
    pub public_flags: u32,
}

impl MessageRow {
    pub const SELECT_FROM: &'static str = r#"
        SELECT
            m.id, m.channel_id, m.content, m.timestamp, m.edited_timestamp,
            m.type AS kind, m.flags, m.mention_everyone,
            m.referenced_message_id, m.referenced_channel_id,
            m.referenced_guild_id, m.reference_type,
            u.id AS author_id, u.username, u.discriminator, u.global_name, u.avatar,
            u.bot, u.public_flags
        FROM messages m
        INNER JOIN users u ON u.id = m.author_id
    "#;
}

#[derive(Debug, Clone, Serialize)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub author: PartialUser,
    #[serde(rename = "type")]
    pub kind: u32,
    pub flags: u32,
    pub content: String,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub timestamp: u64,
    #[serde(serialize_with = "serialize_millis_iso_opt")]
    pub edited_timestamp: Option<u64>,
    pub pinned: bool,
    pub mention_everyone: bool,
    pub tts: bool,
    pub mentions: Vec<PartialUser>,
    pub mention_roles: Vec<String>,
    pub embeds: Vec<serde_json::Value>,
    pub attachments: Vec<serde_json::Value>,
    pub stickers: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_reference: Option<MessageReference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referenced_message: Option<Option<Box<Message>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<Box<ThreadResponse>>,
}

impl From<MessageRow> for Message {
    fn from(row: MessageRow) -> Self {
        let message_reference = row
            .referenced_channel_id
            .map(|channel_id| MessageReference {
                channel_id,
                message_id: row.referenced_message_id,
                guild_id: row.referenced_guild_id,
                kind: row
                    .reference_type
                    .unwrap_or(MessageReferenceType::Default as u8),
            });
        let referenced_message = if row.kind == MessageType::Reply as u32 {
            Some(None)
        } else {
            None
        };

        Self {
            id: row.id,
            channel_id: row.channel_id,
            author: PartialUser {
                id: row.author_id,
                username: row.username,
                discriminator: row.discriminator,
                global_name: row.global_name,
                avatar: row.avatar,
                avatar_color: None,
                bot: row.bot,
                flags: row.public_flags,
            },
            kind: row.kind,
            flags: row.flags,
            content: row.content.unwrap_or_default(),
            timestamp: row.timestamp,
            edited_timestamp: row.edited_timestamp,
            pinned: false,
            mention_everyone: row.mention_everyone,
            tts: false,
            mentions: vec![],
            mention_roles: vec![],
            embeds: vec![],
            attachments: vec![],
            stickers: vec![],
            message_reference,
            nonce: None,
            referenced_message,
            thread: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(kind: u32, ref_id: Option<&str>) -> MessageRow {
        MessageRow {
            id: "100".to_string(),
            channel_id: "200".to_string(),
            content: Some("hello".to_string()),
            timestamp: 1_700_000_000_000,
            edited_timestamp: None,
            kind,
            flags: 0,
            mention_everyone: false,
            referenced_message_id: ref_id.map(str::to_string),
            referenced_channel_id: ref_id.map(|_| "200".to_string()),
            referenced_guild_id: ref_id.map(|_| "1".to_string()),
            reference_type: ref_id.map(|_| 0),
            author_id: "300".to_string(),
            username: "alice".to_string(),
            discriminator: "0000".to_string(),
            global_name: None,
            avatar: None,
            bot: false,
            public_flags: 0,
        }
    }

    #[test]
    fn message_serializes_discord_compatible_fields_and_reply_states() {
        let normal = serde_json::to_value(Message::from(sample_row(0, None))).unwrap();
        assert_eq!(normal["type"], 0);
        assert_eq!(normal["content"], "hello");
        assert_eq!(normal["author"]["id"], "300");
        assert!(normal.get("nonce").is_none());
        assert!(normal.get("message_reference").is_none());
        assert!(normal.get("referenced_message").is_none());
        assert!(normal["author"].get("bot").is_none());

        let deleted_target_reply =
            serde_json::to_value(Message::from(sample_row(19, Some("99")))).unwrap();
        assert_eq!(deleted_target_reply["type"], 19);
        assert_eq!(
            deleted_target_reply["message_reference"]["message_id"],
            "99"
        );
        assert_eq!(deleted_target_reply["message_reference"]["type"], 0);
        assert_eq!(
            deleted_target_reply.get("referenced_message"),
            Some(&serde_json::Value::Null)
        );

        let mut hydrated_reply = Message::from(sample_row(19, Some("99")));
        hydrated_reply.referenced_message =
            Some(Some(Box::new(Message::from(sample_row(0, None)))));
        let hydrated_val = serde_json::to_value(hydrated_reply).unwrap();
        assert_eq!(hydrated_val["referenced_message"]["id"], "100");
    }
}
