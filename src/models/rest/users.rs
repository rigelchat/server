use serde::{Deserialize, Serialize};

use crate::models::serde_helpers::deserialize_double_option;
use crate::models::user::UserProfileData;

#[derive(Deserialize)]
pub struct UpdateUserRequest {
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub global_name: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub avatar: Option<Option<String>>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub banner: Option<Option<String>>,
}

#[derive(Deserialize)]
pub struct UpdateProfileRequest {
    pub bio: Option<String>,
    pub pronouns: Option<String>,
    pub accent_color: Option<u32>,
    pub theme_colors: Option<Vec<Option<u32>>>,
}

#[derive(Serialize)]
pub struct UserProfileResponse {
    pub user_profile: UserProfileData,
}

#[derive(Deserialize)]
pub struct UpdateSettingsRequest {
    pub status: Option<String>,
    pub locale: Option<String>,
    pub theme: Option<String>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub background_gradient_preset: Option<Option<String>>,
    pub developer_mode: Option<bool>,
    pub synced_preferences: Option<String>,
}