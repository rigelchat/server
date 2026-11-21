use serde::{Deserialize, Serialize};

use crate::constants::limits::{MAX_CUSTOM_STATUS_TEXT_LENGTH, MAX_EMOJI_NAME_LENGTH};
use crate::error::{AppError, ValidationCode};
use crate::models::serde_helpers::{deserialize_double_option, parse_timestamp_iso};
use crate::models::user::{CustomStatus, GuildFolder, PartialUser, UserProfileData};
use crate::utils::snowflake::validate_snowflake;

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

#[derive(Debug, Deserialize)]
pub struct CustomStatusInput {
    pub text: Option<String>,
    pub expires_at: Option<serde_json::Value>,
    pub emoji_id: Option<String>,
    pub emoji_name: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    pub status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub custom_status: Option<Option<CustomStatusInput>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub status_resets_at: Option<Option<serde_json::Value>>,
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

impl CustomStatusInput {
    pub fn try_into_custom_status(self) -> Result<CustomStatus, AppError> {
        let text = self
            .text
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(ref t) = text {
            if t.chars().count() > MAX_CUSTOM_STATUS_TEXT_LENGTH {
                return Err(AppError::validation(
                    "custom_status.text",
                    ValidationCode::ContentExceedsMaxLength,
                    "Custom status text must not exceed 128 characters.",
                ));
            }
        }

        let emoji_name = self
            .emoji_name
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(ref name) = emoji_name {
            if name.chars().count() > MAX_EMOJI_NAME_LENGTH {
                return Err(AppError::validation(
                    "custom_status.emoji_name",
                    ValidationCode::StringLengthInvalid,
                    "Emoji name must not exceed 64 characters.",
                ));
            }
        }

        let emoji_id = self
            .emoji_id
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        if let Some(ref id) = emoji_id {
            validate_snowflake("custom_status.emoji_id", id)?;
        }

        let expires_at = self
            .expires_at
            .map(|v| parse_timestamp_iso("custom_status.expires_at", v))
            .transpose()?
            .flatten();

        Ok(CustomStatus {
            text,
            expires_at,
            emoji_id,
            emoji_name,
            emoji_animated: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_status_validation_checks_text_emoji_and_expiry() {
        let ok = CustomStatusInput {
            text: Some("  Coding  ".to_string()),
            expires_at: Some(serde_json::json!("2026-12-31T23:59:59Z")),
            emoji_id: Some("123456789012345678".to_string()),
            emoji_name: Some("rocket".to_string()),
        }
        .try_into_custom_status()
        .unwrap();
        assert_eq!(ok.text.as_deref(), Some("Coding"));

        let too_long = CustomStatusInput {
            text: Some("x".repeat(MAX_CUSTOM_STATUS_TEXT_LENGTH + 1)),
            expires_at: None,
            emoji_id: None,
            emoji_name: None,
        }
        .try_into_custom_status();
        assert!(too_long.is_err());

        let bad_id = CustomStatusInput {
            text: None,
            expires_at: None,
            emoji_id: Some("not-a-snowflake".to_string()),
            emoji_name: None,
        }
        .try_into_custom_status();
        assert!(bad_id.is_err());
    }
}
