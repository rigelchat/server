use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::constants::flags::UserFlags;
use crate::models::serde_helpers::millis_to_iso;

#[derive(Debug, Clone, FromRow)]
pub struct UserRow {
    pub id: String,
    pub created_at: u64,
    pub bot: bool,
    pub public_flags: u32,
    pub username: String,
    pub email: String,
    pub verified: bool,
    pub password_hash: Option<String>,
    pub global_name: Option<String>,
    pub discriminator: String,
    pub avatar: Option<String>,
    pub banner: Option<String>,
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct PublicUser {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub bot: bool,
    #[sqlx(rename = "public_flags")]
    pub flags: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayUserSummary {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub avatar_color: Option<u32>,
    pub bot: bool,
    pub flags: u32,
}

impl From<&UserRow> for GatewayUserSummary {
    fn from(u: &UserRow) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            discriminator: u.discriminator.clone(),
            global_name: u.global_name.clone(),
            avatar: u.avatar.clone(),
            avatar_color: None,
            bot: u.bot,
            flags: u.public_flags,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentUser {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub avatar_color: Option<u32>,
    pub accent_color: Option<u32>,
    pub banner: Option<String>,
    pub banner_color: Option<String>,
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub email: Option<String>,
    pub email_bounced: bool,
    pub phone: Option<String>,
    pub verified: bool,
    pub has_verified_phone: bool,
    pub mfa_enabled: bool,
    pub authenticator_types: Vec<u8>,
    pub nsfw_allowed: bool,
    pub flags: u32,
    pub is_staff: bool,
    pub acls: Vec<String>,
    pub required_actions: Vec<String>,
    pub traits: Vec<String>,
    pub premium_type: u8,
    pub premium_since: Option<String>,
    pub premium_until: Option<String>,
    pub premium_billing_cycle: Option<String>,
    pub premium_lifetime_sequence: Option<u64>,
    pub premium_grace_ends_at: Option<String>,
    pub premium_badge_masked: bool,
    pub premium_badge_hidden: bool,
    pub premium_badge_sequence_hidden: bool,
    pub premium_badge_timestamp_hidden: bool,
    pub premium_discriminator: bool,
    pub premium_enabled_override: bool,
    pub premium_perks_disabled: bool,
    pub premium_purchase_disabled: bool,
    pub premium_will_cancel: bool,
    pub has_dismissed_premium_onboarding: bool,
    pub has_ever_purchased: bool,
    pub has_unread_gift_inventory: bool,
    pub unread_gift_inventory_count: u32,
    pub pending_bulk_message_deletion: Option<String>,
    pub last_voice_activity_sharing_change_at: Option<String>,
    pub password_last_changed_at: Option<String>,
    pub privacy_agreed_at: Option<String>,
    pub terms_agreed_at: Option<String>,
}

impl CurrentUser {
    pub fn from_row(user: UserRow, profile: Option<UserProfileRow>) -> Self {
        let created_iso = millis_to_iso(user.created_at);
        let is_staff = UserFlags::from_bits_truncate(user.public_flags).contains(UserFlags::STAFF);

        Self {
            id: user.id,
            username: user.username,
            discriminator: user.discriminator,
            global_name: user.global_name,
            avatar: user.avatar,
            avatar_color: None,
            accent_color: profile.as_ref().and_then(|p| p.accent_color),
            banner: user.banner,
            banner_color: None,
            bio: profile.as_ref().and_then(|p| p.bio.clone()),
            pronouns: profile.and_then(|p| p.pronouns),
            email: Some(user.email),
            email_bounced: false,
            phone: None,
            verified: user.verified,
            has_verified_phone: false,
            mfa_enabled: false,
            authenticator_types: vec![],
            nsfw_allowed: true,
            flags: user.public_flags,
            is_staff,
            acls: vec![],
            required_actions: vec![],
            traits: vec![],
            premium_type: 0,
            premium_since: None,
            premium_until: None,
            premium_billing_cycle: None,
            premium_lifetime_sequence: None,
            premium_grace_ends_at: None,
            premium_badge_masked: false,
            premium_badge_hidden: false,
            premium_badge_sequence_hidden: false,
            premium_badge_timestamp_hidden: false,
            premium_discriminator: false,
            premium_enabled_override: false,
            premium_perks_disabled: false,
            premium_purchase_disabled: false,
            premium_will_cancel: false,
            has_dismissed_premium_onboarding: false,
            has_ever_purchased: false,
            has_unread_gift_inventory: false,
            unread_gift_inventory_count: 0,
            pending_bulk_message_deletion: None,
            last_voice_activity_sharing_change_at: None,
            password_last_changed_at: Some(created_iso.clone()),
            privacy_agreed_at: Some(created_iso.clone()),
            terms_agreed_at: Some(created_iso),
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct UserProfileRow {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub accent_color: Option<u32>,
    pub theme_color_primary: Option<u32>,
    pub theme_color_secondary: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserProfileData {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub accent_color: Option<u32>,
    pub theme_colors: Option<[u32; 2]>,
}

impl From<UserProfileRow> for UserProfileData {
    fn from(row: UserProfileRow) -> Self {
        let theme_colors = match (row.theme_color_primary, row.theme_color_secondary) {
            (Some(primary), Some(secondary)) => Some([primary, secondary]),
            _ => None,
        };

        Self {
            bio: row.bio,
            pronouns: row.pronouns,
            accent_color: row.accent_color,
            theme_colors,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct UserSettingsRow {
    pub status: String,
    pub afk_timeout: u32,
    pub locale: String,
    pub theme: String,
    pub background_gradient_preset: Option<String>,
    pub developer_mode: bool,
    pub synced_preferences: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettings {
    pub status: String,
    pub custom_status: Option<serde_json::Value>,
    pub status_resets_at: Option<String>,
    pub status_resets_to: Option<String>,
    pub locale: String,
    pub theme: String,
    pub afk_timeout: u32,
    pub developer_mode: bool,
    pub synced_preferences: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background_gradient_preset: Option<String>,
    pub trusted_domains: Vec<String>,
    pub render_embeds: bool,
    pub render_reactions: bool,
    pub render_spoilers: u8,
    pub inline_attachment_media: bool,
    pub inline_embed_media: bool,
    pub animate_emoji: bool,
    pub animate_stickers: u8,
    pub gif_auto_play: bool,
    pub message_display_compact: bool,
    pub time_format: u8,
    pub enable_tts_command: bool,
    pub privacy_setup_version: u32,
    pub privacy_setup_completed_at: Option<String>,
    pub restricted_guilds: Vec<String>,
    pub bot_restricted_guilds: Vec<String>,
    pub default_guilds_restricted: bool,
    pub bot_default_guilds_restricted: bool,
    pub friend_source_flags: u32,
    pub incoming_call_flags: u32,
    pub group_dm_add_permission_flags: u32,
    pub profile_privacy: u32,
    pub default_share_voice_activity: bool,
    pub default_hide_muted_channels: bool,
    pub sensitive_content_friend_dm_filter: u8,
    pub sensitive_content_non_friend_dm_filter: u8,
    pub sensitive_content_guild_filter: u8,
    pub suppress_unprivileged_self_mentions: bool,
    pub suppress_unprivileged_self_mentions_bypass_user_ids: Vec<String>,
    pub staff_dm_access_user_ids: Vec<String>,
    pub guild_folders: Vec<serde_json::Value>,
}

impl Default for UserSettings {
    fn default() -> Self {
        Self {
            status: "online".to_string(),
            custom_status: None,
            status_resets_at: None,
            status_resets_to: None,
            locale: "en-US".to_string(),
            theme: "dark".to_string(),
            afk_timeout: 600,
            developer_mode: false,
            synced_preferences: String::new(),
            background_gradient_preset: None,
            trusted_domains: vec![],
            render_embeds: true,
            render_reactions: true,
            render_spoilers: 1,
            inline_attachment_media: true,
            inline_embed_media: true,
            animate_emoji: true,
            animate_stickers: 0,
            gif_auto_play: true,
            message_display_compact: false,
            time_format: 0,
            enable_tts_command: true,
            privacy_setup_version: 1,
            privacy_setup_completed_at: Some(
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ),
            restricted_guilds: vec![],
            bot_restricted_guilds: vec![],
            default_guilds_restricted: false,
            bot_default_guilds_restricted: false,
            friend_source_flags: 7,
            incoming_call_flags: 8,
            group_dm_add_permission_flags: 8,
            profile_privacy: 0,
            default_share_voice_activity: true,
            default_hide_muted_channels: false,
            sensitive_content_friend_dm_filter: 0,
            sensitive_content_non_friend_dm_filter: 2,
            sensitive_content_guild_filter: 1,
            suppress_unprivileged_self_mentions: false,
            suppress_unprivileged_self_mentions_bypass_user_ids: vec![],
            staff_dm_access_user_ids: vec![],
            guild_folders: vec![],
        }
    }
}

impl From<UserSettingsRow> for UserSettings {
    fn from(row: UserSettingsRow) -> Self {
        Self {
            status: row.status,
            afk_timeout: row.afk_timeout,
            locale: row.locale,
            theme: row.theme,
            background_gradient_preset: row.background_gradient_preset,
            developer_mode: row.developer_mode,
            synced_preferences: row.synced_preferences.unwrap_or_default(),
            ..Default::default()
        }
    }
}