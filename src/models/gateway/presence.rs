use serde::{Deserialize, Serialize};

use crate::models::serde_helpers::deserialize_double_option;
use crate::models::user::{CustomStatus, CustomStatusInput, PartialUser};

#[derive(Debug, Deserialize)]
pub struct PresenceUpdatePayload {
    pub status: String,
    pub afk: Option<bool>,
    pub mobile: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub custom_status: Option<Option<CustomStatusInput>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum PresenceUser {
    Partial(PartialUser),
    IdOnly { id: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct GatewayPresence {
    pub user: PresenceUser,
    pub status: String,
    pub mobile: bool,
    pub afk: bool,
    pub custom_status: Option<CustomStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
}
