use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::models::serde_helpers::{serialize_millis_iso, serialize_millis_iso_opt};
use crate::models::user::PartialUser;

#[derive(Debug, Clone, FromRow)]
pub struct MessageDbRow {
    pub id: String,
    pub channel_id: String,
    pub content: Option<String>,
    pub timestamp: u64,
    pub edited_timestamp: Option<u64>,
    pub kind: u32,
    pub flags: u32,
    pub author_id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub bot: bool,
    pub public_flags: u32,
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
    pub nonce: Option<String>,
}

impl From<MessageDbRow> for Message {
    fn from(row: MessageDbRow) -> Self {
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
            mention_everyone: false,
            tts: false,
            mentions: vec![],
            mention_roles: vec![],
            embeds: vec![],
            attachments: vec![],
            stickers: vec![],
            nonce: None,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageType {
    Default = 0,
    RecipientAdd = 1,
    RecipientRemove = 2,
    Call = 3,
    ChannelNameChange = 4,
    ChannelIconChange = 5,
    ChannelPinnedMessage = 6,
    UserJoin = 7,
    ChannelFollowAdd = 12,
    ThreadCreated = 18,
    Reply = 19,
    ThreadStarterMessage = 21,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_serializes_discord_compatible_fields() {
        let msg = Message::from(MessageDbRow {
            id: "100".to_string(),
            channel_id: "200".to_string(),
            content: Some("hello".to_string()),
            timestamp: 1_700_000_000_000,
            edited_timestamp: None,
            kind: 0,
            flags: 0,
            author_id: "300".to_string(),
            username: "alice".to_string(),
            discriminator: "0000".to_string(),
            global_name: None,
            avatar: None,
            bot: false,
            public_flags: 0,
        });

        let val = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], 0);
        assert_eq!(val["content"], "hello");
        assert_eq!(val["author"]["id"], "300");
        assert!(val.get("nonce").is_none());
        assert!(val["author"].get("bot").is_none());
    }
}
