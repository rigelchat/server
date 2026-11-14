use serde::Serialize;
use sqlx::FromRow;

use crate::models::serde_helpers::serialize_millis_iso;
use crate::models::user::PublicUser;

#[derive(Debug, Clone, FromRow)]
pub struct BaseGuildMember {
    pub joined_at: u64,
    pub guild_id: String,
    pub user_id: String,
    pub nick: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct GuildMemberRole {
    pub guild_id: String,
    pub user_id: String,
    pub role_id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberUserRef {
    pub id: String,
}

/// Compact member format used in READY / GUILD_CREATE (when user is in READY.users)
#[derive(Debug, Clone, Serialize)]
pub struct GatewayGuildMember {
    pub user: GuildMemberUserRef,
    pub roles: Vec<String>,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub accent_color: Option<u32>,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
    pub mute: bool,
    pub deaf: bool,
    pub communication_disabled_until: Option<String>,
}

/// Full member format returned by REST `/guilds/{id}/members` and GUILD_MEMBER_ADD
#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberResponse {
    pub user: PublicUser,
    pub roles: Vec<String>,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub premium_since: Option<String>,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
    pub deaf: bool,
    pub mute: bool,
    pub flags: u32,
    pub pending: bool,
}