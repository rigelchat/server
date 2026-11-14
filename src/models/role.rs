use serde::Serialize;
use sqlx::FromRow;

use crate::models::serde_helpers::serialize_u64_as_string;

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct Role {
    pub id: String,
    pub guild_id: String,
    pub position: u32,
    pub name: String,
    pub color: u32,
    pub unicode_emoji: Option<String>,
    pub hoist: bool,
    pub mentionable: bool,
    #[serde(serialize_with = "serialize_u64_as_string")]
    pub permissions: u64,
}