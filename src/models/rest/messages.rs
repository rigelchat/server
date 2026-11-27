use serde::Deserialize;

use crate::models::serde_helpers::{deserialize_opt_string_or_int, deserialize_string_or_int};

#[derive(Debug, Clone, Deserialize)]
pub struct MessageReferenceInput {
    #[serde(deserialize_with = "deserialize_string_or_int")]
    pub message_id: String,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
    pub channel_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
    pub guild_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: Option<u8>,
    pub fail_if_not_exists: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AllowedMentionsInput {
    pub parse: Option<Vec<String>>,
    pub users: Option<Vec<String>>,
    pub roles: Option<Vec<String>>,
    pub replied_user: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub content: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
    pub nonce: Option<String>,
    pub message_reference: Option<MessageReferenceInput>,
    pub allowed_mentions: Option<AllowedMentionsInput>,
    pub flags: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct EditMessageRequest {
    pub content: Option<String>,
    pub allowed_mentions: Option<AllowedMentionsInput>,
    pub flags: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GetMessagesQuery {
    pub limit: Option<u32>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub around: Option<String>,
}
