use serde::{Deserialize, Serialize};

use crate::models::user::PartialUser;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: Option<String>,
    pub login: Option<String>,
    pub password: String,
    pub invite_code: Option<String>,
}

impl LoginRequest {
    pub fn identifier(&self) -> Option<&str> {
        self.login
            .as_deref()
            .or(self.email.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: String,
    pub user: PartialUser,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: Option<String>,
    pub email: Option<String>,
    pub password: Option<String>,
    pub global_name: Option<String>,
    pub theme: Option<String>,
    pub invite_code: Option<String>,
}

#[derive(Deserialize)]
pub struct UsernameAvailabilityQuery {
    pub username: String,
}

#[derive(Serialize)]
pub struct UsernameAvailabilityResponse {
    pub available: bool,
}

#[derive(Deserialize)]
pub struct UsernameSuggestionsRequest {
    pub global_name: String,
}

#[derive(Serialize)]
pub struct UsernameSuggestionsResponse {
    pub suggestions: Vec<String>,
}
