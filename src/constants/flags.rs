use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// See: https://docs.discord.com/developers/resources/user#user-object-user-flags
    /// See: https://docs.fluxer.app/http-api/users/#public-user-flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct UserFlags: u32 {
        const STAFF = 1 << 0;
        const PARTNERED = 1 << 1;
        const BUG_HUNTER = 1 << 3;
        const FRIENDLY_BOT = 1 << 4;
        const FRIENDLY_BOT_MANUAL_APPROVAL = 1 << 5;
        const SPAMMER = 1 << 6;
        const PROFILE_HIDDEN = 1 << 7;
    }
}

bitflags! {
    /// See: https://docs.discord.com/developers/resources/message#message-object-message-types
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