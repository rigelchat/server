use serde::Serialize;
use sqlx::FromRow;

use crate::models::channel::ChannelResponse;
use crate::models::guild_member::GatewayGuildMember;
use crate::models::role::Role;
use crate::models::serde_helpers::serialize_millis_iso;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct BaseGuild {
    pub id: String,
    pub created_at: u64,
    pub owner_id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub description: Option<String>,
    pub rules_channel_id: Option<String>,
    pub vanity_url_code: Option<String>,
    pub afk_channel_id: Option<String>,
    pub afk_timeout: u32,
    pub system_channel_id: Option<String>,
    pub system_channel_flags: u32,
    pub discoverable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayGuildProperties {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub splash: Option<String>,
    pub owner_id: String,
    pub features: Vec<String>,
    pub system_channel_id: Option<String>,
    pub system_channel_flags: u32,
    pub afk_channel_id: Option<String>,
    pub afk_timeout: u32,
    pub rules_channel_id: Option<String>,
    pub vanity_url_code: Option<String>,
    pub verification_level: u8,
    pub mfa_level: u8,
    pub nsfw: bool,
    pub nsfw_level: u8,
    pub default_message_notifications: u8,
    pub explicit_content_filter: u8,
    pub disabled_operations: u8,
    pub message_history_cutoff: Option<String>,
}

impl From<BaseGuild> for GatewayGuildProperties {
    fn from(g: BaseGuild) -> Self {
        Self {
            id: g.id,
            name: g.name,
            icon: g.icon,
            banner: g.banner,
            splash: None,
            owner_id: g.owner_id,
            features: vec![],
            system_channel_id: g.system_channel_id,
            system_channel_flags: g.system_channel_flags,
            afk_channel_id: g.afk_channel_id,
            afk_timeout: g.afk_timeout,
            rules_channel_id: g.rules_channel_id,
            vanity_url_code: g.vanity_url_code,
            verification_level: 0,
            mfa_level: 0,
            nsfw: false,
            nsfw_level: 0,
            default_message_notifications: 0,
            explicit_content_filter: 0,
            disabled_operations: 0,
            message_history_cutoff: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayGuild {
    pub id: String,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
    pub member_count: u64,
    pub online_count: u64,
    pub channels: Vec<ChannelResponse>,
    pub roles: Vec<Role>,
    pub members: Vec<GatewayGuildMember>,
    pub emojis: Vec<serde_json::Value>,
    pub stickers: Vec<serde_json::Value>,
    pub presences: Vec<serde_json::Value>,
    pub voice_states: Vec<serde_json::Value>,
    pub threads: Vec<serde_json::Value>,
    pub properties: GatewayGuildProperties,
}