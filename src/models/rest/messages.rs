use serde::Deserialize;

use crate::models::serde_helpers::deserialize_opt_string_or_int;

#[derive(Debug, Deserialize)]
pub struct CreateMessageRequest {
    pub content: Option<String>,
    #[serde(default, deserialize_with = "deserialize_opt_string_or_int")]
    pub nonce: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct EditMessageRequest {
    pub content: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct GetMessagesQuery {
    pub limit: Option<u32>,
    pub before: Option<String>,
    pub after: Option<String>,
    pub around: Option<String>,
}
