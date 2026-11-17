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

#[derive(Debug, Clone, Serialize)]
pub struct MessageMember {
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub accent_color: Option<u32>,
    pub roles: Vec<String>,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
    pub mute: bool,
    pub deaf: bool,
    pub communication_disabled_until: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayMessageEvent {
    #[serde(flatten)]
    pub message: Message,
    pub channel_type: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<MessageMember>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GetMessagesQuery {
    pub limit: Option<u32>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub around: Option<String>,
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