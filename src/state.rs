use std::{
    collections::{HashMap, HashSet},
    env,
    sync::Arc,
};

use serde::Serialize;
use sqlx::mysql::MySqlPool;
use tokio::sync::RwLock;

use crate::gateway::session::GatewaySession;

#[derive(Clone, Debug)]
pub struct InstanceConfig {
    pub name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub color: Option<String>,
    pub contact_email: Option<String>,
    pub owner_id: Option<String>,
    pub website_url: Option<String>,
    pub terms_url: Option<String>,
    pub privacy_url: Option<String>,
    pub webapp_url: Option<String>,
}

#[derive(Clone, Debug)]
pub struct AppConfig {
    pub auth_secret: String,
    pub public_http_url: String,
    pub public_ws_url: String,
    pub instance: InstanceConfig,
}

impl AppConfig {
    pub fn from_env() -> Self {
        Self {
            auth_secret: require_env("AUTH_SECRET"),
            public_http_url: require_env("PUBLIC_HTTP_URL"),
            public_ws_url: require_env("PUBLIC_WS_URL"),
            instance: InstanceConfig {
                name: require_env("INSTANCE_NAME"),
                description: optional_env("INSTANCE_DESCRIPTION"),
                image: optional_env("INSTANCE_IMAGE"),
                color: optional_env("INSTANCE_COLOR"),
                contact_email: optional_env("INSTANCE_CONTACT_EMAIL"),
                owner_id: optional_env("INSTANCE_OWNER_ID"),
                website_url: optional_env("INSTANCE_WEBSITE_URL"),
                terms_url: optional_env("INSTANCE_TERMS_URL"),
                privacy_url: optional_env("INSTANCE_PRIVACY_URL"),
                webapp_url: optional_env("INSTANCE_WEBAPP_URL"),
            },
        }
    }
}

fn optional_env(key: &str) -> Option<String> {
    env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn require_env(key: &str) -> String {
    optional_env(key).unwrap_or_else(|| panic!("Environment variable '{key}' must be set"))
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: MySqlPool,
    pub sessions: Arc<RwLock<HashMap<String, GatewaySession>>>,
}

impl AppState {
    pub async fn online_user_ids(&self) -> HashSet<String> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter_map(|s| s.user_id.clone())
            .collect()
    }

    pub async fn add_user_guild(&self, user_id: &str, guild_id: &str) {
        let mut sessions = self.sessions.write().await;
        for session in sessions.values_mut() {
            if session.user_id.as_deref() == Some(user_id) {
                session.guilds.insert(guild_id.to_string());
            }
        }
    }

    pub async fn remove_user_guild(&self, user_id: &str, guild_id: &str) {
        let mut sessions = self.sessions.write().await;
        for session in sessions.values_mut() {
            if session.user_id.as_deref() == Some(user_id) {
                session.guilds.remove(guild_id);
            }
        }
    }

    pub async fn dispatch_user<T: Serialize>(&self, event_name: &str, data: &T, user_id: &str) {
        let value = match serde_json::to_value(data) {
            Ok(v) => v,
            Err(err) => {
                tracing::error!(error = %err, event = %event_name, "Failed to serialize payload");
                return;
            }
        };

        let sessions = self.sessions.read().await;
        for session in sessions.values() {
            if session.user_id.as_deref() == Some(user_id) {
                session.dispatch(event_name, value.clone()).await;
            }
        }
    }

    pub async fn dispatch_guild<T: Serialize>(&self, event_name: &str, data: &T, guild_id: &str) {
        let value = match serde_json::to_value(data) {
            Ok(v) => v,
            Err(err) => {
                tracing::error!(error = %err, event = %event_name, "Failed to serialize payload");
                return;
            }
        };

        let sessions = self.sessions.read().await;
        for session in sessions.values() {
            if session.guilds.contains(guild_id) {
                session.dispatch(event_name, value.clone()).await;
            }
        }
    }
}