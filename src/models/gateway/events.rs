use serde::{Deserialize, Serialize};

use crate::models::gateway::presence::GatewayPresence;
use crate::models::guild_member::{GuildMemberFields, GuildMemberResponse};
use crate::models::message::Message;
use crate::models::role::Role;
use crate::models::serde_helpers::serialize_millis_iso;
use crate::models::thread::{ThreadMember, ThreadResponse};

pub type MessageMember = GuildMemberFields;

#[derive(Debug, Clone, Serialize)]
pub struct GuildMemberAddEvent {
    pub guild_id: String,
    #[serde(flatten)]
    pub member: GuildMemberResponse,
}

pub type GuildMemberUpdateEvent = GuildMemberAddEvent;

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

#[derive(Debug, Clone, Serialize)]
pub struct GuildRoleEvent<'a> {
    pub guild_id: &'a str,
    pub role: &'a Role,
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildRoleBulkEvent<'a> {
    pub guild_id: &'a str,
    pub roles: &'a [Role],
}

#[derive(Debug, Clone, Serialize)]
pub struct GuildRoleDeleteEvent<'a> {
    pub guild_id: &'a str,
    pub role_id: &'a str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadCreateEvent {
    #[serde(flatten)]
    pub thread: ThreadResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub newly_created: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadDeleteEvent<'a> {
    pub id: &'a str,
    pub guild_id: &'a str,
    pub parent_id: &'a str,
    #[serde(rename = "type")]
    pub kind: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadListSyncEvent {
    pub guild_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_ids: Option<Vec<String>>,
    pub threads: Vec<ThreadResponse>,
    pub members: Vec<ThreadMember>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadMemberUpdateEvent {
    pub guild_id: String,
    #[serde(flatten)]
    pub member: ThreadMember,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadMembersUpdateEvent {
    pub id: String,
    pub guild_id: String,
    pub member_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_members: Option<Vec<ThreadMember>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_member_ids: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadMemberListItem {
    pub user_id: String,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub join_timestamp: u64,
    pub flags: u32,
    pub member: Option<GuildMemberResponse>,
    pub presence: GatewayPresence,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadMemberListUpdateEvent {
    pub guild_id: String,
    pub thread_id: String,
    pub members: Vec<ThreadMemberListItem>,
}

#[derive(Debug, Deserialize)]
pub struct RequestForumUnreadsPayload {
    pub guild_id: String,
    pub channel_id: String,
    pub threads: Vec<ForumUnreadRequestItem>,
}

#[derive(Debug, Deserialize)]
pub struct ForumUnreadRequestItem {
    pub thread_id: String,
    pub ack_message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForumUnreadEntry {
    pub thread_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub missing: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForumUnreadsEvent {
    pub guild_id: String,
    pub channel_id: String,
    pub threads: Vec<ForumUnreadEntry>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageAckEvent {
    pub channel_id: String,
    pub message_id: String,
    pub version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mention_count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manual: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
}
