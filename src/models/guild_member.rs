use serde::Serialize;
use sqlx::FromRow;

use crate::models::serde_helpers::serialize_millis_iso;
use crate::models::user::PartialUser;

#[derive(Debug, Clone, FromRow)]
pub struct BaseGuildMember {
    pub joined_at: u64,
    pub nick: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct GuildMemberRole {
    pub user_id: String,
    pub role_id: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct GuildMemberUserRow {
    pub user_id: String,
    pub joined_at: u64,
    pub nick: Option<String>,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub bot: bool,
    pub public_flags: u32,
}

impl GuildMemberUserRow {
    pub fn into_partial_user(self) -> PartialUser {
        PartialUser {
            id: self.user_id,
            username: self.username,
            discriminator: self.discriminator,
            global_name: self.global_name,
            avatar: self.avatar,
            avatar_color: None,
            bot: self.bot,
            flags: self.public_flags,
        }
    }

    pub fn into_member_response(self, roles: Vec<String>) -> GuildMemberResponse {
        let joined_at = self.joined_at;
        let nick = self.nick.clone();
        GuildMemberResponse {
            user: self.into_partial_user(),
            member: GuildMemberFields::new(nick, roles, joined_at),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberUserRef {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberFields {
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

impl GuildMemberFields {
    pub fn new(nick: Option<String>, roles: Vec<String>, joined_at: u64) -> Self {
        Self {
            nick,
            avatar: None,
            banner: None,
            accent_color: None,
            roles,
            joined_at,
            mute: false,
            deaf: false,
            communication_disabled_until: None,
        }
    }
}

/// Compact member format used in READY.
#[derive(Debug, Clone, Serialize)]
pub struct GatewayGuildMember {
    pub user: GuildMemberUserRef,
    #[serde(flatten)]
    pub member: GuildMemberFields,
}

/// Full member format returned by REST `/guilds/{id}/members` and GUILD_MEMBER_ADD.
#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberResponse {
    pub user: PartialUser,
    #[serde(flatten)]
    pub member: GuildMemberFields,
}