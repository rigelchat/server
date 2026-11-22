use serde::Deserialize;

use crate::models::serde_helpers::{deserialize_double_option, deserialize_opt_u64_string_or_int};

#[derive(Debug, Deserialize)]
pub struct CreateRoleRequest {
    pub name: Option<String>,
    pub color: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub permissions: Option<u64>,
    pub hoist: Option<bool>,
    pub mentionable: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub unicode_emoji: Option<Option<String>>,
    pub icon_source_role_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub color: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_opt_u64_string_or_int")]
    pub permissions: Option<u64>,
    pub hoist: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub hoist_position: Option<Option<i32>>,
    pub mentionable: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub icon: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub unicode_emoji: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub struct RolePositionUpdate {
    pub id: String,
    pub position: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct RoleHoistPositionUpdate {
    pub id: String,
    pub hoist_position: i32,
}
