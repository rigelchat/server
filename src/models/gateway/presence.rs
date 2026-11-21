use serde::Deserialize;

use crate::models::rest::users::CustomStatusInput;
use crate::models::serde_helpers::deserialize_double_option;
use crate::models::user::{CustomStatus, PartialUser};

#[derive(Debug, Deserialize)]
pub struct PresenceUpdatePayload {
    pub status: String,
    pub afk: Option<bool>,
    pub mobile: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub custom_status: Option<Option<CustomStatusInput>>,
}

pub fn serialize_presence_user(user_id: &str, user: Option<&PartialUser>) -> serde_json::Value {
    user.and_then(|u| serde_json::to_value(u).ok())
        .unwrap_or_else(|| serde_json::json!({ "id": user_id }))
}

pub fn build_presence_value(
    fluxer_user_val: &serde_json::Value,
    status: &str,
    mobile: bool,
    afk: bool,
    custom_status: Option<&CustomStatus>,
    guild_id: Option<&str>,
) -> serde_json::Value {
    let mut obj = serde_json::json!({
        "user": fluxer_user_val,
        "status": status,
        "mobile": mobile,
        "afk": afk,
        "custom_status": custom_status,
    });
    if let Some(gid) = guild_id {
        obj["guild_id"] = serde_json::json!(gid);
    }
    obj
}