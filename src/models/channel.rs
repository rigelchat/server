use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::constants::guild::{DEFAULT_VOICE_BITRATE, DEFAULT_VOICE_CONNECTION_LIMIT};
use crate::models::serde_helpers::serialize_u64_as_string;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GuildChannel {
    pub id: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub kind: u32,
    pub position: u32,
    pub guild_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub last_message_id: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ChannelOverwriteRow {
    pub channel_id: String,
    #[sqlx(rename = "type")]
    pub kind: u32,
    pub target_id: String,
    pub allow: u64,
    pub deny: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PermissionOverwrite {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u32,
    #[serde(serialize_with = "serialize_u64_as_string")]
    pub allow: u64,
    #[serde(serialize_with = "serialize_u64_as_string")]
    pub deny: u64,
}

impl From<ChannelOverwriteRow> for PermissionOverwrite {
    fn from(row: ChannelOverwriteRow) -> Self {
        Self {
            id: row.target_id,
            kind: row.kind,
            allow: row.allow,
            deny: row.deny,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelResponse {
    #[serde(flatten)]
    pub base: GuildChannel,
    pub topic: Option<String>,
    pub nsfw: bool,
    pub nsfw_override: Option<bool>,
    pub content_warning_level: u8,
    pub content_warning_text: Option<String>,
    pub rate_limit_per_user: u32,
    pub last_pin_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_connection_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtc_region: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtc_p2p: Option<bool>,
    pub permission_overwrites: Vec<PermissionOverwrite>,
}

impl ChannelResponse {
    pub fn from_parts(base: GuildChannel, permission_overwrites: Vec<PermissionOverwrite>) -> Self {
        let is_voice = base.kind == ChannelType::GuildVoice as u32;
        Self {
            base,
            topic: None,
            nsfw: false,
            nsfw_override: None,
            content_warning_level: 0,
            content_warning_text: None,
            rate_limit_per_user: 0,
            last_pin_timestamp: None,
            bitrate: is_voice.then_some(DEFAULT_VOICE_BITRATE),
            user_limit: is_voice.then_some(0),
            voice_connection_limit: is_voice.then_some(DEFAULT_VOICE_CONNECTION_LIMIT),
            rtc_region: is_voice.then_some(None),
            rtc_p2p: is_voice.then_some(false),
            permission_overwrites,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelType {
    GuildText = 0,
    Dm = 1,
    GuildVoice = 2,
    GroupDm = 3,
    GuildCategory = 4,
    GuildAnnouncement = 5,
}