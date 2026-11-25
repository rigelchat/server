use serde::Deserialize;

use crate::models::serde_helpers::{deserialize_double_option, deserialize_opt_u64_string_or_int};

#[derive(Debug, Clone, Deserialize)]
pub struct PermissionOverwriteInput {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u32,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub allow: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub deny: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct SetPermissionOverwriteRequest {
    #[serde(rename = "type")]
    pub kind: u32,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub allow: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub deny: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ForumTagInput {
    pub id: Option<String>,
    pub name: String,
    pub moderated: Option<bool>,
    pub emoji_id: Option<String>,
    pub emoji_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DefaultReactionInput {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub emoji_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub emoji_name: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannelRequest {
    #[serde(rename = "type")]
    pub kind: u32,
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub topic: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub url: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub parent_id: Option<Option<String>>,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub voice_connection_limit: Option<u32>,
    pub rate_limit_per_user: Option<u32>,
    pub permission_overwrites: Option<Vec<PermissionOverwriteInput>>,
    pub nsfw: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub nsfw_override: Option<Option<bool>>,
    pub content_warning_level: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub content_warning_text: Option<Option<String>>,
    pub flags: Option<u32>,
    pub available_tags: Option<Vec<ForumTagInput>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_reaction_emoji: Option<Option<DefaultReactionInput>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_sort_order: Option<Option<u8>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_forum_layout: Option<Option<u8>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_tag_setting: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_auto_archive_duration: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_thread_rate_limit_per_user: Option<Option<u32>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateChannelRequest {
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<u32>,
    pub position: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub topic: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub url: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub parent_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub bitrate: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub user_limit: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub voice_connection_limit: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub rate_limit_per_user: Option<Option<u32>>,
    pub permission_overwrites: Option<Vec<PermissionOverwriteInput>>,
    pub nsfw: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub nsfw_override: Option<Option<bool>>,
    pub content_warning_level: Option<u8>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub content_warning_text: Option<Option<String>>,
    pub flags: Option<u32>,
    pub available_tags: Option<Vec<ForumTagInput>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_reaction_emoji: Option<Option<DefaultReactionInput>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_sort_order: Option<Option<u8>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_forum_layout: Option<Option<u8>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_tag_setting: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_auto_archive_duration: Option<Option<u32>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub default_thread_rate_limit_per_user: Option<Option<u32>>,
}

#[derive(Debug, Deserialize)]
pub struct ChannelPositionUpdate {
    pub id: String,
    pub position: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub parent_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub preceding_sibling_id: Option<Option<String>>,
    pub lock_permissions: Option<bool>,
}
