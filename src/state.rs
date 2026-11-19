use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use serde::Serialize;
use sqlx::mysql::MySqlPool;
use tokio::sync::RwLock;

use crate::config::AppConfig;
use crate::constants::dialect::ClientDialect;
use crate::gateway::session::GatewaySession;

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

    pub async fn dispatch_user_dialect<F, T>(&self, event_name: &str, user_id: &str, mut build: F)
    where
        F: FnMut(ClientDialect) -> T,
        T: Serialize,
    {
        let sessions = self.sessions.read().await;
        for session in sessions.values() {
            if session.user_id.as_deref() == Some(user_id) {
                let payload = build(session.dialect);
                session.dispatch(event_name, payload).await;
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