#![allow(dead_code)]

use serde::Serialize;

use crate::constants::invite::InviteType;
use crate::models::serde_helpers::serialize_millis_iso;
use crate::models::user::PartialUser;

#[derive(Debug, Clone, Serialize)]
pub struct InviteGuild {
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
    pub features: Vec<String>,
    pub badge_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteChannel {
    pub id: String,
    pub name: Option<String>,
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
    pub inviter: Option<PartialUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_count: Option<usize>,
    pub expires_at: Option<String>,
    pub temporary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreatedInvite {
    #[serde(flatten)]
    pub invite: InviteResponse,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub created_at: u64,
    pub uses: u32,
    pub max_uses: u32,
    pub max_age: u32,
}
