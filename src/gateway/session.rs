use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::{CloseFrame, Message};
use serde::Serialize;
use tokio::sync::mpsc;

use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::Opcode;
use crate::models::gateway::GatewayPayload;

pub struct GatewaySession {
    pub id: String,
    pub dialect: ClientDialect,
    pub user_id: Option<String>,
    pub auth_session_id: Option<String>,
    pub status: String,
    pub afk: bool,
    pub mobile: bool,
    pub sender: mpsc::Sender<Message>,
    pub sequence: AtomicU64,
    pub guilds: HashSet<String>,
    pub member_list_subscriptions: HashMap<String, (String, [i64; 2])>,
}

impl GatewaySession {
    pub fn new(id: String, dialect: ClientDialect, sender: mpsc::Sender<Message>) -> Self {
        Self {
            id,
            dialect,
            user_id: None,
            auth_session_id: None,
            status: "online".to_string(),
            afk: false,
            mobile: false,
            sender,
            sequence: AtomicU64::new(0),
            guilds: HashSet::new(),
            member_list_subscriptions: HashMap::new(),
        }
    }

    pub async fn send(&self, op: Opcode, d: Option<serde_json::Value>) {
        let payload = GatewayPayload::new(op, d);
        self.send_raw(&payload).await;
    }

    pub async fn dispatch<T: Serialize>(&self, event_name: &str, data: T) {
        let value = match serde_json::to_value(data) {
            Ok(v) => v,
            Err(err) => {
                tracing::error!(error = %err, event = %event_name, "Failed to serialize dispatch data");
                return;
            }
        };

        let seq = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let payload = GatewayPayload {
            op: Opcode::Dispatch,
            d: Some(value),
            s: Some(seq),
            t: Some(event_name.to_string()),
        };

        self.send_raw(&payload).await;
    }

    pub async fn close(&self, (code, reason): (u16, &'static str)) {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        let _ = self.sender.send(Message::Close(Some(frame))).await;
    }

    async fn send_raw(&self, payload: &GatewayPayload) {
        let text = match serde_json::to_string(payload) {
            Ok(t) => t,
            Err(err) => {
                tracing::error!(error = %err, "Failed to serialize gateway payload");
                return;
            }
        };

        if self.sender.send(Message::Text(text.into())).await.is_err() {
            tracing::debug!(session_id = %self.id, "Failed to send to gateway client (disconnected)");
        }
    }
}