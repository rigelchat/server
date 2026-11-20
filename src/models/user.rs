use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::FromRow;

use crate::constants::dialect::ClientDialect;
use crate::constants::flags::UserFlags;
use crate::constants::user::DEFAULT_AFK_TIMEOUT;
use crate::models::serde_helpers::millis_to_iso;

#[derive(Debug, Clone, FromRow)]
pub struct UserRow {
    pub id: String,
    pub created_at: u64,
    pub bot: bool,
    pub public_flags: u32,
    pub username: String,
    pub email: Option<String>,
    pub verified: bool,
    pub password_hash: Option<String>,
    pub global_name: Option<String>,
    pub discriminator: String,
    pub avatar: Option<String>,
    pub banner: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialUser {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub avatar_color: Option<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bot: bool,
    pub flags: u32,
}

impl From<&UserRow> for PartialUser {
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
    pub verified: bool,
    pub has_verified_phone: bool,
    pub mfa_enabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bot: bool,
    pub authenticator_types: Vec<u8>,
    pub nsfw_allowed: bool,
    pub flags: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_flags: Option<u32>,
    pub is_staff: bool,
    pub acls: Vec<String>,
    pub required_actions: Vec<String>,
    pub traits: Vec<String>,
    pub timezone: Option<String>,
    pub timezone_privacy_flags: u32,
    pub hidden_profile_badges: Vec<String>,
    pub account_limited: bool,
    pub claimed: bool,
    pub account_tier: String,
    pub verification_need: Option<String>,
    pub has_password: bool,
    pub date_of_birth_required: bool,
    pub profile_restricted: bool,
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
        let has_password = user.password_hash.is_some();

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
            email: user.email,
            email_bounced: false,
            verified: user.verified,
            has_verified_phone: false,
            mfa_enabled: false,
            bot: user.bot,
            authenticator_types: vec![],
            nsfw_allowed: true,
            flags: user.public_flags,
            public_flags: None,
            is_staff,
            acls: vec![],
            required_actions: vec![],
            traits: vec![],
            timezone: None,
            timezone_privacy_flags: 0,
            hidden_profile_badges: vec![],
            account_limited: false,
            claimed: has_password,
            account_tier: if has_password { "verified" } else { "unclaimed" }.to_string(),
            verification_need: if has_password { None } else { Some("password".to_string()) },
            has_password,
            date_of_birth_required: false,
            profile_restricted: false,
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
            password_last_changed_at: has_password.then(|| created_iso.clone()),
            privacy_agreed_at: Some(created_iso.clone()),
            terms_agreed_at: Some(created_iso),
        }
    }

    pub fn with_dialect(mut self, dialect: ClientDialect) -> Self {
        if dialect == ClientDialect::Discord {
            self.public_flags = Some(self.flags);
        }
        self
    }
}

impl From<&CurrentUser> for PartialUser {
    fn from(u: &CurrentUser) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            discriminator: u.discriminator.clone(),
            global_name: u.global_name.clone(),
            avatar: u.avatar.clone(),
            avatar_color: u.avatar_color,
            bot: u.bot,
            flags: u.flags,
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
    pub banner: Option<String>,
    pub banner_color: Option<u32>,
    pub accent_color: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theme_colors: Option<[u32; 2]>,
}

impl From<UserProfileRow> for UserProfileData {
    fn from(row: UserProfileRow) -> Self {
        let theme_colors = match (row.theme_color_primary, row.theme_color_secondary) {
            (Some(primary), Some(secondary)) => Some([primary, secondary]),
            _ => None,
        };

        Self {
            bio: row.bio.filter(|s| !s.is_empty()),
            pronouns: row.pronouns.filter(|s| !s.is_empty()),
            banner: None,
            banner_color: None,
            accent_color: row.accent_color,
            theme_colors,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomStatus {
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_id: Option<String>,
    pub emoji_name: Option<String>,
    pub emoji_animated: bool,
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
    pub extra_settings: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UserSettingsExtras {
    pub custom_status: Option<CustomStatus>,
    pub status_resets_at: Option<String>,
    pub status_resets_to: Option<String>,
    pub regional_format: Option<String>,
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
    pub timestamp_format: u8,
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
    pub allow_mature_content_on_ios: bool,
    pub privacy_setup_version: u32,
    pub privacy_setup_completed_at: Option<String>,
    pub guild_folders: Option<Vec<GuildFolder>>,
}

impl Default for UserSettingsExtras {
    fn default() -> Self {
        Self {
            custom_status: None,
            status_resets_at: None,
            status_resets_to: None,
            regional_format: None,
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
            timestamp_format: 0,
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
            allow_mature_content_on_ios: false,
            privacy_setup_version: 1,
            privacy_setup_completed_at: Some(
                chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            ),
            guild_folders: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserSettings {
    pub status: String,
    pub custom_status: Option<CustomStatus>,
    pub status_resets_at: Option<String>,
    pub status_resets_to: Option<String>,
    pub locale: String,
    pub regional_format: Option<String>,
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
    pub timestamp_format: u8,
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
    pub allow_mature_content_on_ios: bool,
    pub suppress_unprivileged_self_mentions: bool,
    pub suppress_unprivileged_self_mentions_bypass_user_ids: Vec<String>,
    pub staff_dm_access_user_ids: Vec<String>,
    pub guild_folders: Vec<GuildFolder>,
}

impl Default for UserSettings {
    fn default() -> Self {
        let extras = UserSettingsExtras::default();
        Self {
            status: "online".to_string(),
            custom_status: extras.custom_status,
            status_resets_at: extras.status_resets_at,
            status_resets_to: extras.status_resets_to,
            locale: "en-US".to_string(),
            regional_format: extras.regional_format,
            theme: "dark".to_string(),
            afk_timeout: DEFAULT_AFK_TIMEOUT,
            developer_mode: false,
            synced_preferences: String::new(),
            background_gradient_preset: None,
            trusted_domains: extras.trusted_domains,
            render_embeds: extras.render_embeds,
            render_reactions: extras.render_reactions,
            render_spoilers: extras.render_spoilers,
            inline_attachment_media: extras.inline_attachment_media,
            inline_embed_media: extras.inline_embed_media,
            animate_emoji: extras.animate_emoji,
            animate_stickers: extras.animate_stickers,
            gif_auto_play: extras.gif_auto_play,
            message_display_compact: extras.message_display_compact,
            time_format: extras.time_format,
            timestamp_format: extras.timestamp_format,
            privacy_setup_version: extras.privacy_setup_version,
            privacy_setup_completed_at: extras.privacy_setup_completed_at,
            restricted_guilds: extras.restricted_guilds,
            bot_restricted_guilds: extras.bot_restricted_guilds,
            default_guilds_restricted: extras.default_guilds_restricted,
            bot_default_guilds_restricted: extras.bot_default_guilds_restricted,
            friend_source_flags: extras.friend_source_flags,
            incoming_call_flags: extras.incoming_call_flags,
            group_dm_add_permission_flags: extras.group_dm_add_permission_flags,
            profile_privacy: extras.profile_privacy,
            default_share_voice_activity: extras.default_share_voice_activity,
            default_hide_muted_channels: extras.default_hide_muted_channels,
            sensitive_content_friend_dm_filter: extras.sensitive_content_friend_dm_filter,
            sensitive_content_non_friend_dm_filter: extras.sensitive_content_non_friend_dm_filter,
            sensitive_content_guild_filter: extras.sensitive_content_guild_filter,
            allow_mature_content_on_ios: extras.allow_mature_content_on_ios,
            suppress_unprivileged_self_mentions: false,
            suppress_unprivileged_self_mentions_bypass_user_ids: vec![],
            staff_dm_access_user_ids: vec![],
            guild_folders: vec![],
        }
    }
}

pub fn reconcile_guild_folders(folders: &mut Vec<GuildFolder>, actual_guild_ids: &[String]) {
    let actual: HashSet<&str> = actual_guild_ids.iter().map(String::as_str).collect();
    let mut seen: HashSet<String> = HashSet::new();

    folders.retain_mut(|folder| {
        folder
            .guild_ids
            .retain(|gid| actual.contains(gid.as_str()) && seen.insert(gid.clone()));
        folder.id == -1 || !folder.guild_ids.is_empty()
    });

    let missing: Vec<String> = actual_guild_ids
        .iter()
        .filter(|gid| !seen.contains(gid.as_str()))
        .cloned()
        .collect();

    if let Some(uncat) = folders.iter_mut().find(|f| f.id == -1) {
        if !missing.is_empty() {
            let mut updated = missing;
            updated.append(&mut uncat.guild_ids);
            uncat.guild_ids = updated;
        }
    } else {
        folders.insert(
            0,
            GuildFolder {
                id: -1,
                name: None,
                color: 0,
                flags: 0,
                icon: "folder".to_string(),
                guild_ids: missing,
            },
        );
    }
}

impl UserSettings {
    pub fn from_row(row: UserSettingsRow, guild_ids: Vec<String>) -> Self {
        let extras: UserSettingsExtras = row
            .extra_settings
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        let mut guild_folders = extras.guild_folders.unwrap_or_default();
        reconcile_guild_folders(&mut guild_folders, &guild_ids);

        Self {
            status: row.status,
            custom_status: extras.custom_status,
            status_resets_at: extras.status_resets_at,
            status_resets_to: extras.status_resets_to,
            locale: row.locale,
            regional_format: extras.regional_format,
            theme: row.theme,
            afk_timeout: row.afk_timeout,
            developer_mode: row.developer_mode,
            synced_preferences: row.synced_preferences.unwrap_or_default(),
            background_gradient_preset: row.background_gradient_preset,
            trusted_domains: extras.trusted_domains,
            render_embeds: extras.render_embeds,
            render_reactions: extras.render_reactions,
            render_spoilers: extras.render_spoilers,
            inline_attachment_media: extras.inline_attachment_media,
            inline_embed_media: extras.inline_embed_media,
            animate_emoji: extras.animate_emoji,
            animate_stickers: extras.animate_stickers,
            gif_auto_play: extras.gif_auto_play,
            message_display_compact: extras.message_display_compact,
            time_format: extras.time_format,
            timestamp_format: extras.timestamp_format,
            privacy_setup_version: extras.privacy_setup_version,
            privacy_setup_completed_at: extras.privacy_setup_completed_at,
            restricted_guilds: extras.restricted_guilds,
            bot_restricted_guilds: extras.bot_restricted_guilds,
            default_guilds_restricted: extras.default_guilds_restricted,
            bot_default_guilds_restricted: extras.bot_default_guilds_restricted,
            friend_source_flags: extras.friend_source_flags,
            incoming_call_flags: extras.incoming_call_flags,
            group_dm_add_permission_flags: extras.group_dm_add_permission_flags,
            profile_privacy: extras.profile_privacy,
            default_share_voice_activity: extras.default_share_voice_activity,
            default_hide_muted_channels: extras.default_hide_muted_channels,
            sensitive_content_friend_dm_filter: extras.sensitive_content_friend_dm_filter,
            sensitive_content_non_friend_dm_filter: extras.sensitive_content_non_friend_dm_filter,
            sensitive_content_guild_filter: extras.sensitive_content_guild_filter,
            allow_mature_content_on_ios: extras.allow_mature_content_on_ios,
            suppress_unprivileged_self_mentions: false,
            suppress_unprivileged_self_mentions_bypass_user_ids: vec![],
            staff_dm_access_user_ids: vec![],
            guild_folders,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuildFolder {
    pub id: i64,
    pub name: Option<String>,
    pub color: u32,
    pub flags: u32,
    pub icon: String,
    pub guild_ids: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconcile_keeps_valid_dedupes_and_prepends_new() {
        let mut folders = vec![
            GuildFolder {
                id: 1,
                name: Some("Gaming".to_string()),
                color: 0,
                flags: 0,
                icon: "folder".to_string(),
                guild_ids: vec!["g1".to_string(), "g_left".to_string()],
            },
            GuildFolder {
                id: 2,
                name: Some("EmptyAfterLeave".to_string()),
                color: 0,
                flags: 0,
                icon: "folder".to_string(),
                guild_ids: vec!["g_left_2".to_string()],
            },
            GuildFolder {
                id: -1,
                name: None,
                color: 0,
                flags: 0,
                icon: "folder".to_string(),
                guild_ids: vec!["g1".to_string(), "g2".to_string()],
            },
        ];

        let actual = vec!["g1".to_string(), "g2".to_string(), "g3".to_string()];
        reconcile_guild_folders(&mut folders, &actual);

        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].id, 1);
        assert_eq!(folders[0].guild_ids, vec!["g1".to_string()]);
        assert_eq!(folders[1].id, -1);
        assert_eq!(folders[1].guild_ids, vec!["g3".to_string(), "g2".to_string()]);
    }
}
