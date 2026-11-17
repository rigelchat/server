use std::collections::HashMap;

use serde::Serialize;

use crate::models::guild::{DiscordGuildCreate, GatewayGuild};
use crate::models::user::{CurrentUser, PartialUser, UserSettings};

#[derive(Serialize)]
pub struct RtcRegion {
    pub id: String,
    pub name: String,
    pub emoji: String,
}

#[derive(Serialize)]
pub struct ClientSessionInfo {
    pub session_id: String,
    pub status: String,
    pub afk: bool,
    pub mobile: bool,
}

#[derive(Serialize)]
pub struct ReadyData {
    pub auth_session_id_hash: String,
    pub session_id: String,
    pub country_code: String,
    pub version: u8,
    pub guilds: Vec<GatewayGuild>,
    pub private_channels: Vec<serde_json::Value>,
    pub pinned_dms: Vec<String>,
    pub relationships: Vec<serde_json::Value>,
    pub presences: Vec<serde_json::Value>,
    pub read_states: Vec<serde_json::Value>,
    pub notes: HashMap<String, String>,
    pub favorite_memes: Vec<serde_json::Value>,
    pub webauthn_credentials: Vec<serde_json::Value>,
    pub user_guild_settings: Vec<serde_json::Value>,
    pub rtc_regions: Vec<RtcRegion>,
    pub sessions: Vec<ClientSessionInfo>,
    pub user: CurrentUser,
    pub user_settings: UserSettings,
    pub users: Vec<PartialUser>,
}

#[derive(Serialize)]
pub struct UnavailableGuild {
    pub id: String,
    pub unavailable: bool,
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum DiscordReadyGuild {
    Unavailable(UnavailableGuild),
    Full(Box<DiscordGuildCreate>),
}

#[derive(Serialize)]
pub struct PartialApplication {
    pub id: String,
    pub flags: u32,
}

#[derive(Serialize)]
pub struct DiscordReadyData {
    pub v: u8,
    pub user: CurrentUser,
    pub guilds: Vec<DiscordReadyGuild>,
    pub session_id: String,
    pub resume_gateway_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shard: Option<[u32; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub application: Option<PartialApplication>,
}