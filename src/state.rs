use std::sync::Arc;

use serde::Serialize;
use sqlx::mysql::MySqlPool;

use crate::config::AppConfig;
use crate::gateway::hub::GatewayHub;
use crate::gateway::session::GatewaySession;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: MySqlPool,
    pub hub: GatewayHub,
}

impl AppState {
    pub async fn count_guild_online(&self, guild_id: &str) -> u64 {
        self.hub.count_guild_online(guild_id)
    }

    pub async fn add_user_guild(&self, user_id: &str, guild_id: &str) {
        self.hub.add_user_guild(user_id, guild_id);
    }

    pub async fn remove_user_guild(&self, user_id: &str, guild_id: &str) {
        self.hub.remove_user_guild(user_id, guild_id);
    }

    pub fn dispatch_to_sessions<T: Serialize>(
        &self,
        sessions: &[Arc<GatewaySession>],
        event_name: &str,
        data: &T,
    ) {
        if sessions.is_empty() {
            return;
        }
        let Ok(data_json) = serde_json::to_string(data) else {
            tracing::error!(event = %event_name, "Failed to serialize payload");
            return;
        };
        let Ok(event_json) = serde_json::to_string(event_name) else {
            return;
        };

        for session in sessions {
            session.dispatch_pre_serialized(&event_json, &data_json);
        }
    }

    pub async fn dispatch_user<T: Serialize>(&self, event_name: &str, data: &T, user_id: &str) {
        let sessions = self.hub.sessions_for_user(user_id);
        self.dispatch_to_sessions(&sessions, event_name, data);
    }

    pub async fn dispatch_guild<T: Serialize>(&self, event_name: &str, data: &T, guild_id: &str) {
        let sessions = self.hub.sessions_for_guild(guild_id);
        self.dispatch_to_sessions(&sessions, event_name, data);
    }
}