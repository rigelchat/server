use serde::Serialize;

use crate::models::guild_member::{GuildMemberFields, GuildMemberResponse};
use crate::models::message::Message;

pub type MessageMember = GuildMemberFields;

#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberAddEvent {
    pub guild_id: String,
    #[serde(flatten)]
    pub member: GuildMemberResponse,
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

#[derive(Debug, Clone, Serialize)]
pub struct GuildDeleteEvent<'a> {
    pub id: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberRemoveUser<'a> {
    pub id: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberRemoveEvent<'a> {
    pub guild_id: &'a str,
    pub user: GuildMemberRemoveUser<'a>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageDeleteEvent<'a> {
    pub id: &'a str,
    pub channel_id: &'a str,
    pub guild_id: &'a str,
}
