use serde::Deserialize;

use crate::models::serde_helpers::deserialize_double_option;

#[derive(Debug, Deserialize)]
pub struct CreateGuildRequest {
    pub name: String,
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
