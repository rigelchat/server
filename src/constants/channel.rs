#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// See: https://docs.fluxer.app/http-api/channels/#channel-types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChannelType {
    GuildText = 0,
    Dm = 1,
    GuildVoice = 2,
    GroupDm = 3,
    GuildCategory = 4,
    GuildAnnouncement = 5,
    AnnouncementThread = 10,
    PublicThread = 11,
    PrivateThread = 12,
    GuildForum = 15,
    GuildMedia = 16,
    GuildLink = 998,
    DmPersonalNotes = 999,
}

impl ChannelType {
    pub const fn is_thread(kind: u32) -> bool {
        matches!(kind, 10..=12)
    }

    pub const fn is_forum_like(kind: u32) -> bool {
        matches!(kind, 15 | 16)
    }

    pub const fn is_supported_guild_channel(kind: u32) -> bool {
        matches!(kind, 0 | 2 | 4 | 5 | 15 | 16 | 998)
    }

    pub const fn supports_messages(kind: u32) -> bool {
        matches!(kind, 0 | 2 | 5 | 10..=12)
    }

    pub const fn supports_thread_defaults(kind: u32) -> bool {
        matches!(kind, 0 | 5 | 15 | 16)
    }

    pub const fn supports_read_states(kind: u32) -> bool {
        Self::supports_messages(kind) || Self::is_forum_like(kind)
    }

    pub const fn requires_channel_threads_capability(kind: u32) -> bool {
        Self::is_thread(kind) || Self::is_forum_like(kind)
    }
}

/// See: https://docs.fluxer.app/http-api/channels/#permission-overwrite-types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverwriteType {
    Role = 0,
    Member = 1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "u8")]
pub enum ForumSortOrder {
    LatestActivity = 0,
    CreationTime = 1,
}

impl ForumSortOrder {
    pub const fn is_valid(value: u8) -> bool {
        matches!(value, 0 | 1)
    }
}

impl From<ForumSortOrder> for u8 {
    fn from(value: ForumSortOrder) -> u8 {
        value as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "u8")]
pub enum ForumLayout {
    Default = 0,
    List = 1,
    Grid = 2,
}

impl ForumLayout {
    pub const fn is_valid(value: u8) -> bool {
        matches!(value, 0..=2)
    }
}

impl From<ForumLayout> for u8 {
    fn from(value: ForumLayout) -> u8 {
        value as u8
    }
}

pub const DEFAULT_AUTO_ARCHIVE_DURATION: u32 = 4320;
pub const AUTO_ARCHIVE_DURATIONS: [u32; 4] = [60, 1440, 4320, 10080];
pub const DEFAULT_FORUM_TAG_SETTING: &str = "match_some";
pub const VALID_FORUM_TAG_SETTINGS: [&str; 2] = ["match_some", "match_all"];
