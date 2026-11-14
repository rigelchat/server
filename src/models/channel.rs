use serde::{Deserialize, Serialize};
use sqlx::FromRow;

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
    pub nsfw: bool,
    pub nsfw_override: bool,
    pub permission_overwrites: Vec<PermissionOverwrite>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelType {
    GuildText = 0,
    GuildVoice = 2,
    GuildCategory = 4,
}