#![allow(dead_code)]

use serde::{Deserialize, Serialize};

pub const HEARTBEAT_INTERVAL: u32 = 41250;

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GatewayEncoding {
    Json,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum GatewayCompression {
    ZstdStream,
}

/// See: https://docs.discord.com/developers/topics/opcodes-and-status-codes#gateway-gateway-close-event-codes
/// See: https://docs.fluxer.app/gateway/opcodes-and-close-codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum Opcode {
    Dispatch = 0,
    Heartbeat = 1,
    Identify = 2,
    PresenceUpdate = 3,
    VoiceStateUpdate = 4,
    VoiceServerPing = 5,
    Resume = 6,
    Reconnect = 7,
    RequestGuildMembers = 8,
    InvalidSession = 9,
    Hello = 10,
    HeartbeatAck = 11,
    GatewayError = 12,
    VoiceSignal = 13,
    LazyRequest = 14,
    RequestGuildCounts = 15,
    RequestChannelMemberCounts = 16,
    RequestForumUnreads = 28,
}

impl TryFrom<u8> for Opcode {
    type Error = String;

    fn try_from(op: u8) -> Result<Self, Self::Error> {
        match op {
            0 => Ok(Self::Dispatch),
            1 => Ok(Self::Heartbeat),
            2 => Ok(Self::Identify),
            3 => Ok(Self::PresenceUpdate),
            4 => Ok(Self::VoiceStateUpdate),
            5 => Ok(Self::VoiceServerPing),
            6 => Ok(Self::Resume),
            7 => Ok(Self::Reconnect),
            8 => Ok(Self::RequestGuildMembers),
            9 => Ok(Self::InvalidSession),
            10 => Ok(Self::Hello),
            11 => Ok(Self::HeartbeatAck),
            12 => Ok(Self::GatewayError),
            13 => Ok(Self::VoiceSignal),
            14 => Ok(Self::LazyRequest),
            15 => Ok(Self::RequestGuildCounts),
            16 => Ok(Self::RequestChannelMemberCounts),
            28 => Ok(Self::RequestForumUnreads),
            _ => Err(format!("Unknown gateway opcode: {op}")),
        }
    }
}

impl From<Opcode> for u8 {
    fn from(op: Opcode) -> u8 {
        op as u8
    }
}

pub mod close_codes {
    pub const UNKNOWN_ERROR: (u16, &'static str) = (4000, "We're not sure what went wrong. Try reconnecting?");
    pub const UNKNOWN_OPCODE: (u16, &'static str) = (4001, "You sent an invalid Gateway opcode or an invalid payload for an opcode. Don't do that!");
    pub const DECODE_ERROR: (u16, &'static str) = (4002, "Invalid Gateway payload.");
    pub const NOT_AUTHENTICATED: (u16, &'static str) = (4003, "You sent us a payload prior to identifying, or this session has been invalidated.");
    pub const AUTHENTICATION_FAILED: (u16, &'static str) = (4004, "The account token sent with your identify payload is incorrect.");
    pub const ALREADY_AUTHENTICATED: (u16, &'static str) = (4005, "You sent more than one identify payload. Don't do that!");
    pub const INVALID_SEQ: (u16, &'static str) = (4007, "The sequence sent when resuming the session was invalid. Reconnect and start a new session.");
    pub const RATE_LIMITED: (u16, &'static str) = (4008, "Woah nelly! You're sending payloads to us too quickly. Slow it down! You will be disconnected on receiving this.");
    pub const SESSION_TIMED_OUT: (u16, &'static str) = (4009, "Your session timed out. Reconnect and start a new one.");
    pub const INVALID_SHARD: (u16, &'static str) = (4010, "You sent us an invalid shard when identifying.");
    pub const SHARDING_REQUIRED: (u16, &'static str) = (4011, "The session would have handled too many guilds - you are required to shard your connection in order to connect.");
    pub const INVALID_API_VERSION: (u16, &'static str) = (4012, "You sent an invalid version for the gateway.");
    pub const INVALID_INTENTS: (u16, &'static str) = (4013, "You sent an invalid intent for a Gateway Intent. You may have incorrectly calculated the bitwise value.");
    pub const DISALLOWED_INTENTS: (u16, &'static str) = (4014, "You sent a disallowed intent for a Gateway Intent. You may have tried to specify an intent that you have not enabled or are not approved for.");
}

pub mod events {
    pub const READY: &str = "READY";
    pub const SESSIONS_REPLACE: &str = "SESSIONS_REPLACE";
    pub const RESUMED: &str = "RESUMED";
    pub const GUILD_CREATE: &str = "GUILD_CREATE";
    pub const GUILD_DELETE: &str = "GUILD_DELETE";
    pub const GUILD_MEMBER_ADD: &str = "GUILD_MEMBER_ADD";
    pub const GUILD_MEMBER_REMOVE: &str = "GUILD_MEMBER_REMOVE";
    pub const GUILD_MEMBER_LIST_UPDATE: &str = "GUILD_MEMBER_LIST_UPDATE";
    pub const MESSAGE_CREATE: &str = "MESSAGE_CREATE";
    pub const MESSAGE_UPDATE: &str = "MESSAGE_UPDATE";
    pub const MESSAGE_DELETE: &str = "MESSAGE_DELETE";
    pub const PRESENCE_UPDATE: &str = "PRESENCE_UPDATE";
    pub const USER_UPDATE: &str = "USER_UPDATE";
    pub const USER_SETTINGS_UPDATE: &str = "USER_SETTINGS_UPDATE";
}