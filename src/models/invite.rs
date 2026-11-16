#![allow(dead_code)]

use serde::Serialize;

use crate::models::serde_helpers::serialize_millis_iso;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(into = "u8")]
pub enum InviteType {
    Guild = 0,
    GroupDm = 1,
    Friend = 2,
}

impl From<InviteType> for u8 {
    fn from(kind: InviteType) -> u8 {
        kind as u8
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteUserRef {
    pub id: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatedInvite {
    pub code: String,
    pub guild_id: String,
    pub channel_id: String,
    pub inviter: InviteUserRef,
    pub uses: u32,
    pub max_uses: u32,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteGuild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub description: Option<String>,
    pub features: Vec<String>,
    pub verification_level: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteChannel {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteResponse {
    pub code: String,
    #[serde(rename = "type")]
    pub kind: InviteType,
    pub guild: InviteGuild,
    pub channel: InviteChannel,
    pub approximate_member_count: i64,
    pub approximate_presence_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct AcceptedInvite {
    pub guild_id: String,
    pub channel_id: String,
}