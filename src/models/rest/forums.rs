use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::models::guild_member::GuildMemberResponse;
use crate::models::message::Message;
use crate::models::serde_helpers::deserialize_double_option;

#[derive(Debug, Deserialize)]
pub struct CreateForumTagRequest {
    pub name: String,
    pub moderated: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub emoji_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub emoji_name: Option<Option<String>>,
}

pub type UpdateForumTagRequest = CreateForumTagRequest;

#[derive(Debug, Deserialize)]
pub struct GetForumPostDataRequest {
    pub thread_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ForumPostDataItem {
    pub owner: Option<GuildMemberResponse>,
    pub first_message: Option<Message>,
}

#[derive(Debug, Serialize)]
pub struct ForumPostDataResponse {
    pub threads: BTreeMap<String, ForumPostDataItem>,
}
