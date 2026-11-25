use serde::Serialize;
use sqlx::FromRow;

use crate::constants::channel::ChannelType;
use crate::models::channel::ChannelResponse;
use crate::models::gateway::presence::GatewayPresence;
use crate::models::guild_member::GuildMemberResponse;
use crate::models::message::Message;
use crate::models::serde_helpers::{serialize_millis_iso, serialize_millis_iso_opt};

#[derive(Debug, Clone, FromRow)]
pub struct ThreadDbRow {
    pub id: String,
    #[sqlx(rename = "type")]
    pub kind: u32,
    pub guild_id: String,
    pub parent_id: String,
    pub owner_id: String,
    pub name: String,
    pub last_message_id: Option<String>,
    pub last_pin_timestamp: Option<u64>,
    pub rate_limit_per_user: u32,
    pub flags: u32,
    pub started_from_message: bool,
    pub archived: bool,
    pub auto_archive_duration: u32,
    pub archive_timestamp: u64,
    pub locked: bool,
    pub invitable: Option<bool>,
    pub create_timestamp: u64,
    pub message_count: u32,
    pub total_message_sent: u32,
    pub member_count: u32,
    pub parent_kind: u32,
}

impl ThreadDbRow {
    pub const SELECT_COLS: &'static str = r#"
        c.id, c.type, c.guild_id, ct.parent_id, ct.owner_id, c.name,
        c.last_message_id, c.last_pin_timestamp, c.rate_limit_per_user, c.flags,
        ct.started_from_message, ct.archived, ct.auto_archive_duration, ct.archive_timestamp,
        ct.locked, ct.invitable, ct.create_timestamp, ct.message_count,
        ct.total_message_sent, ct.member_count,
        p.type AS parent_kind
    "#;
}

#[derive(Debug, Clone, FromRow)]
pub struct ThreadMemberDbRow {
    pub thread_id: String,
    pub user_id: String,
    pub join_timestamp: u64,
    pub flags: u32,
    pub muted: bool,
    pub mute_config: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadMetadata {
    pub archived: bool,
    pub auto_archive_duration: u32,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub archive_timestamp: u64,
    pub locked: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invitable: Option<bool>,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub create_timestamp: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadMember {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub join_timestamp: u64,
    pub flags: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_config: Option<Option<serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<Option<GuildMemberResponse>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence: Option<Option<GatewayPresence>>,
}

impl ThreadMember {
    pub fn from_db_self(row: ThreadMemberDbRow, include_ids: bool) -> Self {
        let mute_config = row
            .mute_config
            .as_deref()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok());
        Self {
            id: include_ids.then_some(row.thread_id),
            user_id: include_ids.then_some(row.user_id),
            join_timestamp: row.join_timestamp,
            flags: row.flags,
            muted: Some(row.muted),
            mute_config: Some(mute_config),
            member: None,
            presence: None,
        }
    }

    pub fn from_db_public(row: ThreadMemberDbRow, member: Option<GuildMemberResponse>) -> Self {
        Self {
            id: Some(row.thread_id),
            user_id: Some(row.user_id),
            join_timestamp: row.join_timestamp,
            flags: row.flags,
            muted: None,
            mute_config: None,
            member: member.map(Some),
            presence: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ThreadResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u32,
    pub guild_id: String,
    pub parent_id: String,
    pub owner_id: String,
    pub name: String,
    pub last_message_id: Option<String>,
    #[serde(serialize_with = "serialize_millis_iso_opt")]
    pub last_pin_timestamp: Option<u64>,
    pub rate_limit_per_user: u32,
    pub flags: u32,
    pub thread_metadata: ThreadMetadata,
    pub message_count: u32,
    pub total_message_sent: u32,
    pub member_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied_tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_ids_preview: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<ThreadMember>,
}

impl ThreadResponse {
    pub fn from_row(
        row: ThreadDbRow,
        applied_tags: Vec<String>,
        member_ids_preview: Vec<String>,
        member: Option<ThreadMember>,
    ) -> Self {
        let is_forum_post = ChannelType::is_forum_like(row.parent_kind);
        let is_private = row.kind == ChannelType::PrivateThread as u32;
        Self {
            id: row.id,
            kind: row.kind,
            guild_id: row.guild_id,
            parent_id: row.parent_id,
            owner_id: row.owner_id,
            name: row.name,
            last_message_id: row.last_message_id,
            last_pin_timestamp: row.last_pin_timestamp,
            rate_limit_per_user: row.rate_limit_per_user,
            flags: row.flags,
            thread_metadata: ThreadMetadata {
                archived: row.archived,
                auto_archive_duration: row.auto_archive_duration,
                archive_timestamp: row.archive_timestamp,
                locked: row.locked,
                invitable: is_private.then(|| row.invitable.unwrap_or(true)),
                create_timestamp: row.create_timestamp,
            },
            message_count: row.message_count,
            total_message_sent: row.total_message_sent,
            member_count: row
                .member_count
                .min(crate::constants::limits::MAX_THREAD_MEMBER_COUNT_CAP),
            applied_tags: is_forum_post.then_some(applied_tags),
            member_ids_preview: is_forum_post.then_some(member_ids_preview),
            member,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ForumPostResponse {
    #[serde(flatten)]
    pub thread: ThreadResponse,
    pub message: Message,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum ChannelOrThreadResponse {
    Channel(Box<ChannelResponse>),
    Thread(Box<ThreadResponse>),
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum StartThreadResponse {
    ForumPost(Box<ForumPostResponse>),
    Thread(Box<ThreadResponse>),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(kind: u32, parent_kind: u32) -> ThreadDbRow {
        ThreadDbRow {
            id: "100".into(),
            kind,
            guild_id: "1".into(),
            parent_id: "50".into(),
            owner_id: "10".into(),
            name: "Bug report".into(),
            last_message_id: Some("100".into()),
            last_pin_timestamp: None,
            rate_limit_per_user: 0,
            flags: 0,
            started_from_message: false,
            archived: false,
            auto_archive_duration: 4320,
            archive_timestamp: 1_700_000_000_000,
            locked: false,
            invitable: Some(true),
            create_timestamp: 1_700_000_000_000,
            message_count: 2,
            total_message_sent: 2,
            member_count: 80,
            parent_kind,
        }
    }

    #[test]
    fn thread_response_serializes_forum_and_private_fields_only_when_applicable() {
        let forum_post = serde_json::to_value(ThreadResponse::from_row(
            sample_row(ChannelType::PublicThread as u32, ChannelType::GuildForum as u32),
            vec!["t1".into()],
            vec!["10".into()],
            None,
        ))
        .unwrap();
        assert_eq!(forum_post["member_count"], 50);
        assert_eq!(forum_post["applied_tags"], serde_json::json!(["t1"]));
        assert_eq!(forum_post["member_ids_preview"], serde_json::json!(["10"]));
        assert!(forum_post["thread_metadata"].get("invitable").is_none());

        let priv_thread = serde_json::to_value(ThreadResponse::from_row(
            sample_row(ChannelType::PrivateThread as u32, ChannelType::GuildText as u32),
            vec![],
            vec![],
            None,
        ))
        .unwrap();
        assert_eq!(priv_thread["thread_metadata"]["invitable"], true);
        assert!(priv_thread.get("applied_tags").is_none());
        assert!(priv_thread.get("member_ids_preview").is_none());
    }
}
