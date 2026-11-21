use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::ws::{CloseFrame, Message};
use serde::Serialize;
use tokio::sync::{mpsc, watch};

use crate::constants::gateway::{Opcode, close_codes};
use crate::constants::user::PresenceStatus;
use crate::gateway::hub::now_millis;
use crate::models::gateway::GatewayPayload;
use crate::models::gateway::ready::ClientSessionInfo;

struct SessionState {
    user_id: Option<String>,
    auth_session_id: Option<String>,
    status: PresenceStatus,
    afk: bool,
    mobile: bool,
    guilds: HashSet<String>,
    member_list_subscriptions: HashMap<String, (String, [i64; 2])>,
}

pub struct GatewaySession {
    pub id: String,
    sender: mpsc::Sender<Message>,
    sequence: AtomicU64,
    connected_at_ms: u64,
    last_heartbeat_ms: AtomicU64,
    close_tx: watch::Sender<bool>,
    state: RwLock<SessionState>,
}

impl GatewaySession {
    pub fn new(id: String, sender: mpsc::Sender<Message>) -> Self {
        let (close_tx, _) = watch::channel(false);
        let now = now_millis();
        Self {
            id,
            sender,
            sequence: AtomicU64::new(0),
            connected_at_ms: now,
            last_heartbeat_ms: AtomicU64::new(now),
            close_tx,
            state: RwLock::new(SessionState {
                user_id: None,
                auth_session_id: None,
                status: PresenceStatus::Online,
                afk: false,
                mobile: false,
                guilds: HashSet::new(),
                member_list_subscriptions: HashMap::new(),
            }),
        }
    }

    pub fn touch_heartbeat(&self) {
        self.last_heartbeat_ms
            .store(now_millis(), Ordering::Relaxed);
    }

    pub fn connected_at_ms(&self) -> u64 {
        self.connected_at_ms
    }

    pub fn last_heartbeat_ms(&self) -> u64 {
        self.last_heartbeat_ms.load(Ordering::Relaxed)
    }

    pub fn auth_session_id(&self) -> Option<String> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .auth_session_id
            .clone()
    }

    pub fn is_authenticated(&self) -> bool {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .user_id
            .is_some()
    }

    pub fn user_id(&self) -> Option<String> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .user_id
            .clone()
    }

    pub fn guilds(&self) -> HashSet<String> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .guilds
            .clone()
    }

    pub fn has_guild(&self, guild_id: &str) -> bool {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .guilds
            .contains(guild_id)
    }

    pub fn add_guild(&self, guild_id: &str) {
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .guilds
            .insert(guild_id.to_string());
    }

    pub fn remove_guild(&self, guild_id: &str) {
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .guilds
            .remove(guild_id);
    }

    pub fn set_authenticated(
        &self,
        user_id: String,
        auth_session_id: String,
        status: PresenceStatus,
        afk: bool,
        mobile: bool,
        guilds: HashSet<String>,
    ) {
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        st.user_id = Some(user_id);
        st.auth_session_id = Some(auth_session_id);
        st.status = status;
        st.afk = afk;
        st.mobile = mobile;
        st.guilds = guilds;
    }

    pub fn update_presence(
        &self,
        status: PresenceStatus,
        afk: Option<bool>,
        mobile: Option<bool>,
    ) -> Option<(String, HashSet<String>)> {
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        let uid = st.user_id.clone()?;
        st.status = status;
        if let Some(a) = afk {
            st.afk = a;
        }
        if let Some(m) = mobile {
            st.mobile = m;
        }
        Some((uid, st.guilds.clone()))
    }

    pub fn set_status(&self, status: PresenceStatus) {
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .status = status;
    }

    pub fn snapshot_presence(&self) -> (PresenceStatus, bool, bool) {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        (st.status, st.afk, st.mobile)
    }

    pub fn client_session_info(&self) -> ClientSessionInfo {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        ClientSessionInfo {
            session_id: self.id.clone(),
            status: st.status.as_str().to_string(),
            afk: st.afk,
            mobile: st.mobile,
        }
    }

    pub fn set_member_list_subscription(
        &self,
        guild_id: String,
        channel_id: String,
        range: [i64; 2],
    ) {
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .member_list_subscriptions
            .insert(guild_id, (channel_id, range));
    }

    pub fn remove_member_list_subscription(&self, guild_id: &str) {
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .member_list_subscriptions
            .remove(guild_id);
    }

    pub fn member_list_subscription(&self, guild_id: &str) -> Option<(String, [i64; 2])> {
        self.state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .member_list_subscriptions
            .get(guild_id)
            .cloned()
    }

    pub async fn wait_for_close(&self) {
        let mut rx = self.close_tx.subscribe();
        let _ = rx.wait_for(|&closed| closed).await;
    }

    pub fn send(&self, op: Opcode, d: Option<serde_json::Value>) {
        let payload = GatewayPayload::new(op, d);
        self.send_raw(&payload);
    }

    pub fn dispatch<T: Serialize>(&self, event_name: &str, data: T) {
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

        self.send_raw(&payload);
    }

    pub fn dispatch_pre_serialized(&self, event_json: &str, data_json: &str) {
        let seq = self.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let text = format!(r#"{{"op":0,"d":{data_json},"s":{seq},"t":{event_json}}}"#);
        self.send_text(text);
    }

    pub fn close(&self, (code, reason): (u16, &'static str)) {
        let frame = CloseFrame {
            code,
            reason: reason.into(),
        };
        if self.sender.try_send(Message::Close(Some(frame))).is_err() {
            self.trigger_force_close();
        }
    }

    fn trigger_force_close(&self) {
        let _ = self.close_tx.send(true);
    }

    fn send_raw(&self, payload: &GatewayPayload) {
        let text = match serde_json::to_string(payload) {
            Ok(t) => t,
            Err(err) => {
                tracing::error!(error = %err, "Failed to serialize gateway payload");
                return;
            }
        };
        self.send_text(text);
    }

    fn send_text(&self, text: String) {
        match self.sender.try_send(Message::Text(text.into())) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                tracing::warn!(session_id = %self.id, "Gateway send queue full, closing slow session");
                self.close(close_codes::UNKNOWN_ERROR);
                self.trigger_force_close();
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::debug!(session_id = %self.id, "Failed to send to gateway client (disconnected)");
            }
        }
    }
}

