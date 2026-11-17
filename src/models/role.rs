use serde::Serialize;
use sqlx::FromRow;

use crate::models::serde_helpers::serialize_u64_as_string;

#[derive(Debug, Clone, FromRow)]
pub struct RoleRow {
    pub id: String,
    pub position: u32,
    pub name: String,
    pub color: u32,
    pub unicode_emoji: Option<String>,
    pub hoist: bool,
    pub mentionable: bool,
    pub permissions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    pub color: u32,
    pub position: u32,
    pub hoist_position: Option<i32>,
    #[serde(serialize_with = "serialize_u64_as_string")]
    pub permissions: u64,
    pub hoist: bool,
    pub mentionable: bool,
    pub icon: Option<String>,
    pub unicode_emoji: Option<String>,
}

impl From<RoleRow> for Role {
    fn from(r: RoleRow) -> Self {
        Self {
            id: r.id,
            name: r.name,
            color: r.color,
            position: r.position,
            hoist_position: None,
            permissions: r.permissions,
            hoist: r.hoist,
            mentionable: r.mentionable,
            icon: None,
            unicode_emoji: r.unicode_emoji,
        }
    }
}