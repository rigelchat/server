use std::collections::{HashMap, HashSet};
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use axum::extract::ws::{CloseFrame, Message};
use serde::Serialize;
use tokio::sync::{Notify, mpsc};

use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::{Opcode, close_codes};
use crate::gateway::hub::now_millis;
use crate::models::gateway::GatewayPayload;
use crate::models::gateway::ready::ClientSessionInfo;

struct SessionState {
    user_id: Option<String>,
    auth_session_id: Option<String>,
    status: String,
    afk: bool,
    mobile: bool,
    guilds: HashSet<String>,
    member_list_subscriptions: HashMap<String, (String, [i64; 2])>,
}

pub struct GatewaySession {
    pub id: String,
    dialect: AtomicU8,
    sender: mpsc::Sender<Message>,
    sequence: AtomicU64,
    last_heartbeat_ms: AtomicU64,
    closed: AtomicBool,
    close_notify: Notify,
    state: RwLock<SessionState>,
}

impl GatewaySession {
    pub fn new(id: String, dialect: ClientDialect, sender: mpsc::Sender<Message>) -> Self {
        Self {
            id,
            dialect: AtomicU8::new(dialect_to_u8(dialect)),
            sender,
            sequence: AtomicU64::new(0),
            last_heartbeat_ms: AtomicU64::new(now_millis()),
            closed: AtomicBool::new(false),
            close_notify: Notify::new(),
            state: RwLock::new(SessionState {
                user_id: None,
                auth_session_id: None,
                status: "online".to_string(),
                afk: false,
                mobile: false,
                guilds: HashSet::new(),
                member_list_subscriptions: HashMap::new(),
            }),
        }
    }

    pub fn dialect(&self) -> ClientDialect {
        match self.dialect.load(Ordering::Relaxed) {
            1 => ClientDialect::Discord,
            _ => ClientDialect::Fluxer,
        }
    }

    pub fn touch_heartbeat(&self) {
        self.last_heartbeat_ms
            .store(now_millis(), Ordering::Relaxed);
    }

    pub fn last_heartbeat_ms(&self) -> u64 {
        self.last_heartbeat_ms.load(Ordering::Relaxed)
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

    #[allow(clippy::too_many_arguments)]
    pub fn set_authenticated(
        &self,
        dialect: ClientDialect,
        user_id: String,
        auth_session_id: String,
        status: String,
        afk: bool,
        mobile: bool,
        guilds: HashSet<String>,
    ) {
        self.dialect.store(dialect_to_u8(dialect), Ordering::Relaxed);
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
        status: String,
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

    pub fn set_status(&self, status: String) {
        self.state
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .status = status;
    }

    pub fn snapshot_presence(&self) -> (String, bool, bool) {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        (st.status.clone(), st.afk, st.mobile)
    }

    pub fn client_session_info(&self) -> ClientSessionInfo {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        ClientSessionInfo {
            session_id: self.id.clone(),
            status: st.status.clone(),
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
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        self.close_notify.notified().await;
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
        self.closed.store(true, Ordering::Release);
        self.close_notify.notify_one();
    }

    fn send_raw(&self, payload: &GatewayPayload) {
        let text = match serde_json::to_string(payload) {
            Ok(t) => t,
            Err(err) => {
                tracing::error!(error = %err, "Failed to serialize gateway payload");
                return;
            }
        };

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

fn dialect_to_u8(dialect: ClientDialect) -> u8 {
    match dialect {
        ClientDialect::Fluxer => 0,
        ClientDialect::Discord => 1,
    }
}