use serde::Serialize;
use sqlx::FromRow;

use crate::constants::channel::ChannelType;
use crate::constants::flags::PermissionFlags;
use crate::constants::guild::{DEFAULT_VOICE_BITRATE, DEFAULT_VOICE_CONNECTION_LIMIT};
use crate::models::serde_helpers::{millis_to_iso, serialize_u64_as_string};

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct GuildChannel {
    pub id: String,
    #[serde(rename = "type")]
    #[sqlx(rename = "type")]
    pub kind: u32,
    pub position: u32,
    pub guild_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub last_message_id: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ChannelRow {
    pub id: String,
    #[sqlx(rename = "type")]
    pub kind: u32,
    pub position: u32,
    pub guild_id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub last_message_id: Option<String>,
    pub last_pin_timestamp: Option<u64>,
    pub topic: Option<String>,
    pub url: Option<String>,
    pub nsfw: bool,
    pub nsfw_override: Option<bool>,
    pub content_warning_level: u8,
    pub content_warning_text: Option<String>,
    pub rate_limit_per_user: u32,
    pub flags: u32,
    pub default_auto_archive_duration: Option<u32>,
    pub default_thread_rate_limit_per_user: u32,
    pub default_reaction_emoji_id: Option<String>,
    pub default_reaction_emoji_name: Option<String>,
    pub default_sort_order: Option<u8>,
    pub default_forum_layout: u8,
    pub default_tag_setting: String,
    pub bitrate: Option<u32>,
    pub user_limit: Option<u32>,
    pub voice_connection_limit: Option<u32>,
}

impl ChannelRow {
    pub const SELECT_COLS: &'static str = "id, type, position, guild_id, parent_id, name, last_message_id, last_pin_timestamp, topic, url, nsfw, nsfw_override, content_warning_level, content_warning_text, rate_limit_per_user, flags, default_auto_archive_duration, default_thread_rate_limit_per_user, default_reaction_emoji_id, default_reaction_emoji_name, default_sort_order, default_forum_layout, default_tag_setting, bitrate, user_limit, voice_connection_limit";
}

#[derive(Debug, Clone, FromRow)]
pub struct ChannelOverwriteRow {
    pub channel_id: String,
    #[sqlx(rename = "type")]
    pub kind: u32,
    pub target_id: String,
    pub allow: u64,
    pub deny: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionOverwrite {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: u32,
    #[serde(serialize_with = "serialize_u64_as_string")]
    pub allow: u64,
    #[serde(serialize_with = "serialize_u64_as_string")]
    pub deny: u64,
}

impl From<ChannelOverwriteRow> for PermissionOverwrite {
    fn from(row: ChannelOverwriteRow) -> Self {
        Self {
            id: row.target_id,
            kind: row.kind,
            allow: row.allow,
            deny: row.deny,
        }
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct ForumTagRow {
    pub id: String,
    pub channel_id: String,
    pub name: String,
    pub moderated: bool,
    pub emoji_id: Option<String>,
    pub emoji_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForumTag {
    pub id: String,
    pub name: String,
    pub moderated: bool,
    pub emoji_id: Option<String>,
    pub emoji_name: Option<String>,
}

impl From<ForumTagRow> for ForumTag {
    fn from(row: ForumTagRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            moderated: row.moderated,
            emoji_id: row.emoji_id,
            emoji_name: row.emoji_name,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DefaultReaction {
    pub emoji_id: Option<String>,
    pub emoji_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelResponse {
    #[serde(flatten)]
    pub base: GuildChannel,
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub nsfw: bool,
    pub nsfw_override: Option<bool>,
    pub content_warning_level: u8,
    pub content_warning_text: Option<String>,
    pub rate_limit_per_user: u32,
    pub last_pin_timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_auto_archive_duration: Option<Option<u32>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_thread_rate_limit_per_user: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_tags: Option<Vec<ForumTag>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_reaction_emoji: Option<Option<DefaultReaction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_sort_order: Option<Option<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_forum_layout: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_tag_setting: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bitrate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_connection_limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtc_region: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rtc_p2p: Option<bool>,
    pub permission_overwrites: Vec<PermissionOverwrite>,
}

impl ChannelResponse {
    pub fn from_row(
        row: ChannelRow,
        permission_overwrites: Vec<PermissionOverwrite>,
        available_tags: Vec<ForumTag>,
    ) -> Self {
        let is_voice = row.kind == ChannelType::GuildVoice as u32;
        let is_link = row.kind == ChannelType::GuildLink as u32;
        let is_forum = row.kind == ChannelType::GuildForum as u32;
        let is_media = row.kind == ChannelType::GuildMedia as u32;
        let is_forum_like = is_forum || is_media;
        let has_thread_defaults = is_forum_like
            || (ChannelType::supports_thread_defaults(row.kind)
                && (row.default_auto_archive_duration.is_some()
                    || row.default_thread_rate_limit_per_user > 0));

        let default_reaction_emoji = if is_forum_like {
            Some(
                match (row.default_reaction_emoji_id, row.default_reaction_emoji_name) {
                    (None, None) => None,
                    (emoji_id, emoji_name) => Some(DefaultReaction {
                        emoji_id,
                        emoji_name,
                    }),
                },
            )
        } else {
            None
        };

        Self {
            base: GuildChannel {
                id: row.id,
                kind: row.kind,
                position: row.position,
                guild_id: row.guild_id,
                parent_id: row.parent_id,
                name: row.name,
                last_message_id: row.last_message_id,
            },
            topic: row.topic,
            url: if is_link { row.url } else { None },
            nsfw: row.nsfw_override.unwrap_or(row.nsfw),
            nsfw_override: row.nsfw_override,
            content_warning_level: row.content_warning_level,
            content_warning_text: row.content_warning_text,
            rate_limit_per_user: row.rate_limit_per_user,
            last_pin_timestamp: row.last_pin_timestamp.map(millis_to_iso),
            flags: is_forum_like.then_some(row.flags),
            default_auto_archive_duration: has_thread_defaults
                .then_some(row.default_auto_archive_duration),
            default_thread_rate_limit_per_user: has_thread_defaults
                .then_some(row.default_thread_rate_limit_per_user),
            available_tags: is_forum_like.then_some(available_tags),
            default_reaction_emoji,
            default_sort_order: is_forum_like.then_some(row.default_sort_order),
            default_forum_layout: is_forum.then_some(row.default_forum_layout),
            default_tag_setting: is_forum_like.then_some(row.default_tag_setting),
            bitrate: is_voice.then(|| row.bitrate.unwrap_or(DEFAULT_VOICE_BITRATE)),
            user_limit: is_voice.then(|| row.user_limit.unwrap_or(0)),
            voice_connection_limit: is_voice.then(|| {
                row.voice_connection_limit
                    .unwrap_or(DEFAULT_VOICE_CONNECTION_LIMIT)
            }),
            rtc_region: is_voice.then_some(None),
            rtc_p2p: is_voice.then_some(false),
            permission_overwrites,
        }
    }

    pub fn from_parts(base: GuildChannel, permission_overwrites: Vec<PermissionOverwrite>) -> Self {
        let is_voice = base.kind == ChannelType::GuildVoice as u32;
        Self {
            base,
            topic: None,
            url: None,
            nsfw: false,
            nsfw_override: None,
            content_warning_level: 0,
            content_warning_text: None,
            rate_limit_per_user: 0,
            last_pin_timestamp: None,
            flags: None,
            default_auto_archive_duration: None,
            default_thread_rate_limit_per_user: None,
            available_tags: None,
            default_reaction_emoji: None,
            default_sort_order: None,
            default_forum_layout: None,
            default_tag_setting: None,
            bitrate: is_voice.then_some(DEFAULT_VOICE_BITRATE),
            user_limit: is_voice.then_some(0),
            voice_connection_limit: is_voice.then_some(DEFAULT_VOICE_CONNECTION_LIMIT),
            rtc_region: is_voice.then_some(None),
            rtc_p2p: is_voice.then_some(false),
            permission_overwrites,
        }
    }

    pub fn mask_thread_permissions(&mut self) {
        let mask = !PermissionFlags::THREAD_GATED_BITS.bits();
        for ow in &mut self.permission_overwrites {
            ow.allow &= mask;
            ow.deny &= mask;
        }
    }

    pub fn strip_thread_fields(&mut self) {
        self.mask_thread_permissions();
        self.default_auto_archive_duration = None;
        self.default_thread_rate_limit_per_user = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_row(kind: ChannelType) -> ChannelRow {
        ChannelRow {
            id: "100".to_string(),
            kind: kind as u32,
            position: 0,
            guild_id: "1".to_string(),
            parent_id: None,
            name: "chan".to_string(),
            last_message_id: None,
            last_pin_timestamp: None,
            topic: None,
            url: Some("https://example.com".to_string()),
            nsfw: false,
            nsfw_override: Some(true),
            content_warning_level: 0,
            content_warning_text: None,
            rate_limit_per_user: 0,
            flags: 0,
            default_auto_archive_duration: None,
            default_thread_rate_limit_per_user: 0,
            default_reaction_emoji_id: None,
            default_reaction_emoji_name: None,
            default_sort_order: None,
            default_forum_layout: 0,
            default_tag_setting: "match_some".to_string(),
            bitrate: None,
            user_limit: None,
            voice_connection_limit: None,
        }
    }

    #[test]
    fn channel_response_serializes_type_specific_fields_only_when_applicable() {
        let text = serde_json::to_value(ChannelResponse::from_row(
            base_row(ChannelType::GuildText),
            vec![],
            vec![],
        ))
        .unwrap();
        assert_eq!(text["nsfw"], true);
        assert!(text.get("url").is_none());
        assert!(text.get("bitrate").is_none());

        let voice = serde_json::to_value(ChannelResponse::from_row(
            base_row(ChannelType::GuildVoice),
            vec![],
            vec![],
        ))
        .unwrap();
        assert_eq!(voice["bitrate"], DEFAULT_VOICE_BITRATE);
        assert_eq!(
            voice["voice_connection_limit"],
            DEFAULT_VOICE_CONNECTION_LIMIT
        );
        assert_eq!(voice["rtc_p2p"], false);
        assert!(voice.get("url").is_none());

        let link = serde_json::to_value(ChannelResponse::from_row(
            base_row(ChannelType::GuildLink),
            vec![],
            vec![],
        ))
        .unwrap();
        assert_eq!(link["url"], "https://example.com");
        assert!(link.get("bitrate").is_none());

        let forum = serde_json::to_value(ChannelResponse::from_row(
            base_row(ChannelType::GuildForum),
            vec![],
            vec![],
        ))
        .unwrap();
        assert_eq!(forum["flags"], 0);
        assert_eq!(forum["default_forum_layout"], 0);
        assert_eq!(forum["default_tag_setting"], "match_some");
        assert!(forum.get("default_reaction_emoji").is_some());

        let media = serde_json::to_value(ChannelResponse::from_row(
            base_row(ChannelType::GuildMedia),
            vec![],
            vec![],
        ))
        .unwrap();
        assert_eq!(media["flags"], 0);
        assert!(media.get("default_forum_layout").is_none());
    }
}
