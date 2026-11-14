use serde::{Deserialize, Serialize};

use crate::models::user::PublicUser;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub email: Option<String>,
    pub login: Option<String>,
    pub password: String,
    pub invite_code: Option<String>,
}

impl LoginRequest {
    pub fn identifier(&self) -> Option<&str> {
        self.email
            .as_deref()
            .or(self.login.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: String,
    pub user: PublicUser,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub global_name: Option<String>,
}