use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::constants::dialect::ClientDialect;
use crate::models::channel::ChannelResponse;
use crate::models::guild_member::{GatewayGuildMember, GuildMemberResponse};
use crate::models::role::Role;
use crate::models::serde_helpers::serialize_millis_iso;

#[derive(Debug, Clone, FromRow)]
pub struct BaseGuild {
    pub id: String,
    pub created_at: u64,
    pub owner_id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub rules_channel_id: Option<String>,
    pub vanity_url_code: Option<String>,
    pub afk_channel_id: Option<String>,
    pub afk_timeout: u32,
    pub system_channel_id: Option<String>,
    pub system_channel_flags: u32,
    pub discoverable: bool,
}

#[derive(Debug, Deserialize)]
pub struct ListGuildsQuery {
    pub before: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
    pub with_counts: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Guild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub banner_width: Option<u32>,
    pub banner_height: Option<u32>,
    pub splash: Option<String>,
    pub splash_width: Option<u32>,
    pub splash_height: Option<u32>,
    pub splash_card_alignment: u8,
    pub embed_splash: Option<String>,
    pub embed_splash_width: Option<u32>,
    pub embed_splash_height: Option<u32>,
    pub vanity_url_code: Option<String>,
    pub owner_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<bool>,
    pub system_channel_id: Option<String>,
    pub system_channel_flags: u32,
    pub rules_channel_id: Option<String>,
    pub afk_channel_id: Option<String>,
    pub afk_timeout: u32,
    pub features: Vec<String>,
    pub badge_ids: Vec<String>,
    pub verification_level: u8,
    pub mfa_level: u8,
    pub nsfw_level: u8,
    pub nsfw: bool,
    pub content_warning_level: u8,
    pub content_warning_text: Option<String>,
    pub explicit_content_filter: u8,
    pub default_message_notifications: u8,
    pub disabled_operations: u8,
    pub message_history_cutoff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<Role>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emojis: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stickers: Option<Vec<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<Vec<ChannelResponse>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub online_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_member_count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approximate_presence_count: Option<u64>,
}

pub type GatewayGuildProperties = Guild;

impl From<BaseGuild> for Guild {
    fn from(g: BaseGuild) -> Self {
        Self {
            id: g.id,
            name: g.name,
            icon: g.icon,
            banner: g.banner,
            banner_width: None,
            banner_height: None,
            splash: None,
            splash_width: None,
            splash_height: None,
            splash_card_alignment: 0,
            embed_splash: None,
            embed_splash_width: None,
            embed_splash_height: None,
            vanity_url_code: g.vanity_url_code,
            owner_id: g.owner_id,
            owner: None,
            system_channel_id: g.system_channel_id,
            system_channel_flags: g.system_channel_flags,
            rules_channel_id: g.rules_channel_id,
            afk_channel_id: g.afk_channel_id,
            afk_timeout: g.afk_timeout,
            features: if g.discoverable { vec!["DISCOVERABLE".to_string()] } else { vec![] },
            badge_ids: vec![],
            verification_level: 0,
            mfa_level: 0,
            nsfw_level: 0,
            nsfw: false,
            content_warning_level: 0,
            content_warning_text: None,
            explicit_content_filter: 0,
            default_message_notifications: 0,
            disabled_operations: 0,
            message_history_cutoff: None,
            permissions: None,
            roles: None,
            emojis: None,
            stickers: None,
            channels: None,
            member_count: None,
            online_count: None,
            approximate_member_count: None,
            approximate_presence_count: None,
        }
    }
}

impl Guild {
    pub fn with_dialect_for_user(mut self, dialect: ClientDialect, user_id: &str) -> Self {
        if dialect == ClientDialect::Discord {
            self.owner = Some(self.owner_id == user_id);
        }
        self
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayGuild<M = GatewayGuildMember> {
    pub id: String,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
    pub member_count: u64,
    pub online_count: u64,
    pub channels: Vec<ChannelResponse>,
    pub roles: Vec<Role>,
    pub members: Vec<M>,
    pub emojis: Vec<serde_json::Value>,
    pub stickers: Vec<serde_json::Value>,
    pub presences: Vec<serde_json::Value>,
    pub voice_states: Vec<serde_json::Value>,
    pub threads: Vec<serde_json::Value>,
    pub properties: GatewayGuildProperties,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiscordGuildCreate {
    #[serde(flatten)]
    pub properties: Guild,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
    pub large: bool,
    pub unavailable: bool,
    pub member_count: u64,
    pub channels: Vec<ChannelResponse>,
    pub roles: Vec<Role>,
    pub members: Vec<GuildMemberResponse>,
    pub emojis: Vec<serde_json::Value>,
    pub stickers: Vec<serde_json::Value>,
    pub presences: Vec<serde_json::Value>,
    pub voice_states: Vec<serde_json::Value>,
    pub threads: Vec<serde_json::Value>,
}

impl From<GatewayGuild<GuildMemberResponse>> for DiscordGuildCreate {
    fn from(g: GatewayGuild<GuildMemberResponse>) -> Self {
        Self {
            properties: g.properties,
            joined_at: g.joined_at,
            large: false,
            unavailable: false,
            member_count: g.member_count,
            channels: g.channels,
            roles: g.roles,
            members: g.members,
            emojis: g.emojis,
            stickers: g.stickers,
            presences: g.presences,
            voice_states: g.voice_states,
            threads: g.threads,
        }
    }
}