use serde::Serialize;

use crate::constants::channel::ChannelType;
use crate::constants::flags::ReadStateFlags;
use crate::models::serde_helpers::serialize_millis_iso_opt;

#[derive(Debug, Clone, Serialize)]
pub struct ReadState {
    pub id: String,
    pub mention_count: u32,
    pub last_message_id: Option<String>,
    #[serde(serialize_with = "serialize_millis_iso_opt")]
    pub last_pin_timestamp: Option<u64>,
    pub version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
}

pub fn read_state_flags_for_channel_type(channel_type: u32) -> Option<u32> {
    if ChannelType::is_thread(channel_type) {
        Some((ReadStateFlags::IS_GUILD_CHANNEL | ReadStateFlags::IS_THREAD).bits())
    } else if ChannelType::is_forum_like(channel_type) {
        Some(ReadStateFlags::IS_GUILD_CHANNEL.bits())
    } else {
        None
    }
}

impl ReadState {
    pub fn from_parts(
        id: String,
        mention_count: u32,
        last_message_id: Option<String>,
        last_pin_timestamp: Option<u64>,
        channel_type: u32,
    ) -> Self {
        Self {
            id,
            mention_count,
            last_message_id,
            last_pin_timestamp,
            version: "0",
            flags: read_state_flags_for_channel_type(channel_type),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_flags_only_for_threads_and_forums() {
        let text = serde_json::to_value(ReadState::from_parts(
            "10".into(),
            3,
            Some("20".into()),
            None,
            ChannelType::GuildText as u32,
        ))
        .unwrap();
        assert_eq!(text["version"], "0");
        assert_eq!(text["mention_count"], 3);
        assert_eq!(text["last_pin_timestamp"], serde_json::Value::Null);
        assert!(text.get("flags").is_none());

        let forum = serde_json::to_value(ReadState::from_parts(
            "11".into(),
            0,
            Some("21".into()),
            Some(1_700_000_000_000),
            ChannelType::GuildForum as u32,
        ))
        .unwrap();
        assert_eq!(forum["flags"], ReadStateFlags::IS_GUILD_CHANNEL.bits());
        assert_eq!(forum["last_pin_timestamp"], "2023-11-14T22:13:20.000Z");

        let thread = serde_json::to_value(ReadState::from_parts(
            "12".into(),
            1,
            Some("22".into()),
            None,
            ChannelType::PublicThread as u32,
        ))
        .unwrap();
        assert_eq!(
            thread["flags"],
            (ReadStateFlags::IS_GUILD_CHANNEL | ReadStateFlags::IS_THREAD).bits()
        );
    }
}
