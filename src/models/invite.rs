#![allow(dead_code)]

use serde::Serialize;

use crate::constants::invite::InviteType;
use crate::models::serde_helpers::serialize_millis_iso;
use crate::models::user::PartialUser;

#[derive(Debug, Clone, Serialize)]
pub struct InviteGuild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub banner_width: Option<u32>,
    pub banner_height: Option<u32>,
    pub splash: Option<String>,
    pub splash_width: Option<u32>,
    pub splash_height: Option<u32>,
    pub splash_card_alignment: u8,
    pub embed_splash: Option<String>,
    pub embed_splash_width: Option<u32>,
    pub embed_splash_height: Option<u32>,
    pub features: Vec<String>,
    pub badge_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteChannel {
    pub id: String,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteResponse {
    pub code: String,
    #[serde(rename = "type")]
    pub kind: InviteType,
    pub guild: InviteGuild,
    pub channel: InviteChannel,
    pub inviter: Option<PartialUser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member_count: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presence_count: Option<usize>,
    pub expires_at: Option<String>,
    pub temporary: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InviteMetadata {
    #[serde(flatten)]
    pub invite: InviteResponse,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub created_at: u64,
    pub uses: u32,
    pub max_uses: u32,
    pub max_age: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invite_metadata_flattens_invite_and_serializes_iso_created_at() {
        let meta = InviteMetadata {
            invite: InviteResponse {
                code: "aB3dEf7h".to_string(),
                kind: InviteType::Guild,
                guild: InviteGuild {
                    id: "100".to_string(),
                    name: "Example guild".to_string(),
                    icon: None,
                    banner: None,
                    banner_width: None,
                    banner_height: None,
                    splash: None,
                    splash_width: None,
                    splash_height: None,
                    splash_card_alignment: 0,
                    embed_splash: None,
                    embed_splash_width: None,
                    embed_splash_height: None,
                    features: vec![],
                    badge_ids: vec![],
                    description: None,
                },
                channel: InviteChannel {
                    id: "200".to_string(),
                    name: Some("general".to_string()),
                    kind: 0,
                },
                inviter: Some(PartialUser {
                    id: "300".to_string(),
                    username: "alice".to_string(),
                    discriminator: "0000".to_string(),
                    global_name: None,
                    avatar: None,
                    avatar_color: None,
                    bot: false,
                    flags: 0,
                }),
                member_count: Some(12),
                presence_count: Some(4),
                expires_at: None,
                temporary: false,
            },
            created_at: 1_700_000_000_000,
            uses: 3,
            max_uses: 10,
            max_age: 86_400,
        };

        let val = serde_json::to_value(&meta).unwrap();
        assert_eq!(val["code"], "aB3dEf7h");
        assert_eq!(val["type"], 0);
        assert_eq!(val["guild"]["id"], "100");
        assert_eq!(val["channel"]["id"], "200");
        assert_eq!(val["inviter"]["id"], "300");
        assert_eq!(val["member_count"], 12);
        assert_eq!(val["presence_count"], 4);
        assert!(val["expires_at"].is_null());
        assert_eq!(val["temporary"], false);
        assert_eq!(val["created_at"], "2023-11-14T22:13:20.000Z");
        assert_eq!(val["uses"], 3);
        assert_eq!(val["max_uses"], 10);
        assert_eq!(val["max_age"], 86_400);
    }
}
