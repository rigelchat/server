use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// See: https://docs.discord.com/developers/resources/user#user-object-user-flags
    /// See: https://docs.fluxer.app/http-api/users/#public-user-flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct UserFlags: u32 {
        const STAFF = 1 << 0;
        const PARTNER = 1 << 2;
        const BUG_HUNTER = 1 << 3;
        const FRIENDLY_BOT = 1 << 4;
        const FRIENDLY_BOT_MANUAL_APPROVAL = 1 << 5;
        const SPAMMER = 1 << 6;
        const PROFILE_HIDDEN = 1 << 7;
    }
}

bitflags! {
    /// See: https://docs.discord.com/developers/topics/permissions#permissions-bitwise-permission-flags
    /// See: https://docs.fluxer.app/http-api/permissions
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct PermissionFlags: u64 {
        const CREATE_INSTANT_INVITE = 1 << 0;
        const KICK_MEMBERS = 1 << 1;
        const BAN_MEMBERS = 1 << 2;
        const ADMINISTRATOR = 1 << 3;
        const MANAGE_CHANNELS = 1 << 4;
        const MANAGE_GUILD = 1 << 5;
        const ADD_REACTIONS = 1 << 6;
        const VIEW_AUDIT_LOG = 1 << 7;
        const PRIORITY_SPEAKER = 1 << 8;
        const STREAM = 1 << 9;
        const VIEW_CHANNEL = 1 << 10;
        const SEND_MESSAGES = 1 << 11;
        const SEND_TTS_MESSAGES = 1 << 12;
        const MANAGE_MESSAGES = 1 << 13;
        const EMBED_LINKS = 1 << 14;
        const ATTACH_FILES = 1 << 15;
        const READ_MESSAGE_HISTORY = 1 << 16;
        const MENTION_EVERYONE = 1 << 17;
        const USE_EXTERNAL_EMOJIS = 1 << 18;
        const CONNECT = 1 << 20;
        const SPEAK = 1 << 21;
        const MUTE_MEMBERS = 1 << 22;
        const DEAFEN_MEMBERS = 1 << 23;
        const MOVE_MEMBERS = 1 << 24;
        const USE_VAD = 1 << 25;
        const CHANGE_NICKNAME = 1 << 26;
        const MANAGE_NICKNAMES = 1 << 27;
        const MANAGE_ROLES = 1 << 28;
        const MANAGE_WEBHOOKS = 1 << 29;
        const MANAGE_EXPRESSIONS = 1 << 30;
        const MANAGE_THREADS = 1 << 34;
        const CREATE_PUBLIC_THREADS = 1 << 35;
        const CREATE_PRIVATE_THREADS = 1 << 36;
        const USE_EXTERNAL_STICKERS = 1 << 37;
        const SEND_MESSAGES_IN_THREADS = 1 << 38;
        const MODERATE_MEMBERS = 1 << 40;
        const CREATE_EXPRESSIONS = 1 << 43;
        const PIN_MESSAGES = 1 << 51;
        const BYPASS_SLOWMODE = 1 << 52;
        const UPDATE_RTC_REGION = 1 << 53;
        const VIEW_CHANNEL_MEMBERS = 1 << 54;
    }
}

impl PermissionFlags {
    pub const DEFAULT_EVERYONE: Self = Self::from_bits_truncate(
        Self::CREATE_INSTANT_INVITE.bits()
            | Self::ADD_REACTIONS.bits()
            | Self::STREAM.bits()
            | Self::VIEW_CHANNEL.bits()
            | Self::SEND_MESSAGES.bits()
            | Self::EMBED_LINKS.bits()
            | Self::ATTACH_FILES.bits()
            | Self::READ_MESSAGE_HISTORY.bits()
            | Self::USE_EXTERNAL_EMOJIS.bits()
            | Self::CONNECT.bits()
            | Self::SPEAK.bits()
            | Self::USE_VAD.bits()
            | Self::CHANGE_NICKNAME.bits()
            | Self::USE_EXTERNAL_STICKERS.bits()
            | Self::VIEW_CHANNEL_MEMBERS.bits(),
    );

    pub const THREAD_GATED_BITS: Self = Self::from_bits_truncate(
        Self::MANAGE_THREADS.bits()
            | Self::CREATE_PUBLIC_THREADS.bits()
            | Self::CREATE_PRIVATE_THREADS.bits()
            | Self::SEND_MESSAGES_IN_THREADS.bits(),
    );
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct ChannelFlags: u32 {
        const PINNED = 1 << 1;
        const REQUIRE_TAG = 1 << 4;
        const HIDE_MEDIA_DOWNLOAD_OPTIONS = 1 << 15;
    }
}

impl ChannelFlags {
    pub const FORUM_ALLOWED: Self = Self::REQUIRE_TAG;
    pub const MEDIA_ALLOWED: Self = Self::from_bits_truncate(
        Self::REQUIRE_TAG.bits() | Self::HIDE_MEDIA_DOWNLOAD_OPTIONS.bits(),
    );
    pub const THREAD_ALLOWED: Self = Self::PINNED;
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct ThreadMemberFlags: u32 {
        const HAS_INTERACTED = 1 << 0;
        const ALL_MESSAGES = 1 << 1;
        const ONLY_MENTIONS = 1 << 2;
        const NO_MESSAGES = 1 << 3;
    }
}

impl ThreadMemberFlags {
    pub const NOTIFICATION_MASK: Self = Self::from_bits_truncate(
        Self::ALL_MESSAGES.bits() | Self::ONLY_MENTIONS.bits() | Self::NO_MESSAGES.bits(),
    );
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct MessageFlags: u32 {
        const CROSSPOSTED = 1 << 0;
        const IS_CROSSPOST = 1 << 1;
        const SUPPRESS_EMBEDS = 1 << 2;
        const SOURCE_MESSAGE_DELETED = 1 << 3;
        const HAS_THREAD = 1 << 5;
        const FAILED_TO_MENTION_SOME_ROLES_IN_THREAD = 1 << 8;
        const SUPPRESS_NOTIFICATIONS = 1 << 12;
        const VOICE_MESSAGE = 1 << 13;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct SessionFlags: u32 {
        const DEBOUNCE_MESSAGE_REACTIONS = 1 << 1;
        const CHANNEL_THREADS = 1 << 2;
        const GUILD_VISITS = 1 << 3;
    }
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct ReadStateFlags: u32 {
        const IS_GUILD_CHANNEL = 1 << 0;
        const IS_THREAD = 1 << 1;
    }
}
