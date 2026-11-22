use serde::Deserialize;

use crate::models::serde_helpers::{
    deserialize_double_option, deserialize_opt_string_or_int,
    deserialize_opt_u64_string_or_int, deserialize_string_or_int,
    deserialize_template_overwrite_type,
};

#[derive(Debug, Deserialize)]
pub struct CreateGuildRequest {
    pub name: String,
    pub template: Option<GuildCreationTemplate>,
}

#[derive(Debug, Deserialize)]
pub struct GuildCreationTemplate {
    pub verification_level: Option<i64>,
    pub default_message_notifications: Option<i64>,
    pub explicit_content_filter: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
    pub system_channel_id: Option<String>,
    pub afk_timeout: Option<i64>,
    pub system_channel_flags: Option<u32>,
    #[serde(default)]
    pub roles: Option<Vec<GuildTemplateRole>>,
    #[serde(default)]
    pub channels: Option<Vec<GuildTemplateChannel>>,
}

#[derive(Debug, Deserialize)]
pub struct GuildTemplateRole {
    #[serde(deserialize_with = "deserialize_string_or_int")]
    pub id: String,
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub permissions: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub permissions_new: Option<u64>,
    pub color: Option<u32>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
    pub unicode_emoji: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct GuildTemplateChannel {
    #[serde(deserialize_with = "deserialize_string_or_int")]
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u32,
    pub name: Option<String>,
    pub topic: Option<String>,
    pub position: Option<i64>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
    pub parent_id: Option<String>,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub voice_connection_limit: Option<u32>,
    pub nsfw: Option<bool>,
    pub rate_limit_per_user: Option<u32>,
    pub permission_overwrites: Option<Vec<GuildTemplateOverwrite>>,
}

#[derive(Debug, Deserialize)]
pub struct GuildTemplateOverwrite {
    #[serde(deserialize_with = "deserialize_string_or_int")]
    pub id: String,
    #[serde(rename = "type", default, deserialize_with = "deserialize_template_overwrite_type")]
    pub kind: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub allow: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub deny: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateGuildRequest {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub banner: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub system_channel_id: Option<Option<String>>,
    pub system_channel_flags: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub rules_channel_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub afk_channel_id: Option<Option<String>>,
    pub afk_timeout: Option<u32>,
    pub default_message_notifications: Option<u8>,
    pub verification_level: Option<u8>,
    pub explicit_content_filter: Option<u8>,
    pub nsfw_level: Option<u8>,
    pub nsfw: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListGuildsQuery {
    pub before: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
    pub with_counts: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCurrentMemberRequest {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub nick: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateGuildMemberRequest {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub nick: Option<Option<String>>,
    pub roles: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct ListMembersQuery {
    pub limit: Option<u32>,
    pub after: Option<String>,
}
