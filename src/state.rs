use std::collections::HashSet;
use std::sync::Arc;

use serde::Serialize;
use sqlx::mysql::MySqlPool;

use crate::config::AppConfig;
use crate::constants::dialect::ClientDialect;
use crate::gateway::hub::GatewayHub;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub db: MySqlPool,
    pub hub: GatewayHub,
}

impl AppState {
    pub async fn online_user_ids(&self) -> HashSet<String> {
        self.hub.online_user_ids()
    }

    pub async fn count_guild_online(&self, guild_id: &str) -> u64 {
        self.hub.count_guild_online(guild_id)
    }

    pub async fn add_user_guild(&self, user_id: &str, guild_id: &str) {
        self.hub.add_user_guild(user_id, guild_id);
    }

    pub async fn remove_user_guild(&self, user_id: &str, guild_id: &str) {
        self.hub.remove_user_guild(user_id, guild_id);
    }

    pub async fn dispatch_user<T: Serialize>(&self, event_name: &str, data: &T, user_id: &str) {
        let value = match serde_json::to_value(data) {
            Ok(v) => v,
            Err(err) => {
                tracing::error!(error = %err, event = %event_name, "Failed to serialize payload");
                return;
            }
        };

        for session in self.hub.sessions_for_user(user_id) {
            session.dispatch(event_name, value.clone());
        }
    }

    pub async fn dispatch_user_dialect<F, T>(&self, event_name: &str, user_id: &str, mut build: F)
    where
        F: FnMut(ClientDialect) -> T,
        T: Serialize,
    {
        for session in self.hub.sessions_for_user(user_id) {
            let payload = build(session.dialect());
            session.dispatch(event_name, payload);
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

        for session in self.hub.sessions_for_guild(guild_id) {
            session.dispatch(event_name, value.clone());
        }
    }
}