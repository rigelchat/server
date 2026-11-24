use serde::{Deserialize, Serialize};

use crate::models::serde_helpers::{TimestampInput, deserialize_double_option};
use crate::models::user::{CustomStatusInput, GuildFolder, PartialUser, UserProfileData};

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub global_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub avatar: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub banner: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub bio: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub pronouns: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub accent_color: Option<Option<u32>>,
}

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub bio: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub pronouns: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub accent_color: Option<Option<u32>>,
    pub theme_colors: Option<Vec<Option<u32>>>,
}

#[derive(Deserialize)]
pub struct GetProfileQuery {
    pub with_mutual_friends: Option<bool>,
    pub with_mutual_guilds: Option<bool>,
}

#[derive(Serialize)]
pub struct MutualGuild {
    pub id: String,
    pub nick: Option<String>,
}

#[derive(Serialize)]
pub struct UserProfileResponse {
    pub user: PartialUser,
    pub user_profile: UserProfileData,
    pub profile_badges: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutual_friends: Option<Vec<PartialUser>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mutual_guilds: Option<Vec<MutualGuild>>,
    pub connected_accounts: Vec<serde_json::Value>,
    pub timezone_offset: Option<i32>,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    pub status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub custom_status: Option<Option<CustomStatusInput>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub status_resets_at: Option<Option<TimestampInput>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub status_resets_to: Option<Option<String>>,
    pub locale: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub regional_format: Option<Option<String>>,
    pub theme: Option<String>,
    pub afk_timeout: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub background_gradient_preset: Option<Option<String>>,
    pub developer_mode: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub synced_preferences: Option<Option<String>>,
    pub inline_attachment_media: Option<bool>,
    pub inline_embed_media: Option<bool>,
    pub gif_auto_play: Option<bool>,
    pub render_embeds: Option<bool>,
    pub render_reactions: Option<bool>,
    pub animate_emoji: Option<bool>,
    pub animate_stickers: Option<u8>,
    pub render_spoilers: Option<u8>,
    pub message_display_compact: Option<bool>,
    pub time_format: Option<u8>,
    pub timestamp_format: Option<u8>,
    pub friend_source_flags: Option<u32>,
    pub flags: Option<u32>,
    pub incoming_call_flags: Option<u32>,
    pub group_dm_add_permission_flags: Option<u32>,
    pub profile_privacy: Option<u32>,
    pub default_hide_muted_channels: Option<bool>,
    pub allow_mature_content_on_ios: Option<bool>,
    pub default_guilds_restricted: Option<bool>,
    pub bot_default_guilds_restricted: Option<bool>,
    pub restricted_guilds: Option<Vec<String>>,
    pub bot_restricted_guilds: Option<Vec<String>>,
    pub trusted_domains: Option<Vec<String>>,
    pub guild_folders: Option<Vec<GuildFolder>>,
}
