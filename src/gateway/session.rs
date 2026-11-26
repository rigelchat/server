use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, RwLock};

use axum::extract::ws::{CloseFrame, Message};
use serde::Serialize;
use tokio::sync::{mpsc, watch};

use crate::constants::flags::SessionFlags;
use crate::constants::gateway::{
    GATEWAY_MAX_EVENTS_PER_WINDOW, GATEWAY_MAX_EXPENSIVE_OPS_PER_WINDOW,
    GATEWAY_PRE_AUTH_MAX_EVENTS, GATEWAY_RATE_LIMIT_WINDOW_SECS, Opcode, close_codes,
};
use crate::constants::user::PresenceStatus;
use crate::gateway::hub::now_millis;
use crate::models::gateway::GatewayPayload;
use crate::models::gateway::ready::ClientSessionInfo;

struct SessionState {
    user_id: Option<String>,
    auth_session_id: Option<String>,
    is_bot: bool,
    flags: u32,
    status: PresenceStatus,
    afk: bool,
    mobile: bool,
    guilds: HashSet<String>,
    active_guilds: HashSet<String>,
    thread_subscriptions: HashSet<String>,
    thread_member_lists: HashMap<String, Vec<String>>,
    member_list_subscriptions: HashMap<String, (String, [i64; 2])>,
}

struct GatewayRateLimiter {
    window_start_ms: u64,
    total_events: u32,
    expensive_events: u32,
    pre_auth_events: u32,
    forum_unreads_timestamps_ms: Vec<u64>,
}

impl GatewayRateLimiter {
    fn new(now_ms: u64) -> Self {
        Self {
            window_start_ms: now_ms,
            total_events: 0,
            expensive_events: 0,
            pre_auth_events: 0,
            forum_unreads_timestamps_ms: Vec::new(),
        }
    }

    fn check_forum_unreads(&mut self, now_ms: u64) -> bool {
        self.forum_unreads_timestamps_ms
            .retain(|&ts| now_ms.saturating_sub(ts) < 5_000);
        if self.forum_unreads_timestamps_ms.len() >= 5 {
            return false;
        }
        self.forum_unreads_timestamps_ms.push(now_ms);
        true
    }

    fn check(&mut self, now_ms: u64, authenticated: bool, is_expensive: bool) -> bool {
        if !authenticated {
            self.pre_auth_events = self.pre_auth_events.saturating_add(1);
            return self.pre_auth_events <= GATEWAY_PRE_AUTH_MAX_EVENTS;
        }

        if now_ms.saturating_sub(self.window_start_ms) >= GATEWAY_RATE_LIMIT_WINDOW_SECS * 1000 {
            self.window_start_ms = now_ms;
            self.total_events = 0;
            self.expensive_events = 0;
        }

        self.total_events = self.total_events.saturating_add(1);
        if self.total_events > GATEWAY_MAX_EVENTS_PER_WINDOW {
            return false;
        }

        if is_expensive {
            self.expensive_events = self.expensive_events.saturating_add(1);
            if self.expensive_events > GATEWAY_MAX_EXPENSIVE_OPS_PER_WINDOW {
                return false;
            }
        }

        true
    }
}

pub struct GatewaySession {
    pub id: String,
    sender: mpsc::Sender<Message>,
    sequence: AtomicU64,
    connected_at_ms: u64,
    last_heartbeat_ms: AtomicU64,
    close_tx: watch::Sender<bool>,
    closed: AtomicBool,
    state: RwLock<SessionState>,
    rate_limiter: Mutex<GatewayRateLimiter>,
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
            closed: AtomicBool::new(false),
            state: RwLock::new(SessionState {
                user_id: None,
                auth_session_id: None,
                is_bot: false,
                flags: 0,
                status: PresenceStatus::Online,
                afk: false,
                mobile: false,
                guilds: HashSet::new(),
                active_guilds: HashSet::new(),
                thread_subscriptions: HashSet::new(),
                thread_member_lists: HashMap::new(),
                member_list_subscriptions: HashMap::new(),
            }),
            rate_limiter: Mutex::new(GatewayRateLimiter::new(now)),
        }
    }

    pub fn check_rate_limit(&self, authenticated: bool, is_expensive: bool) -> bool {
        let now = now_millis();
        self.rate_limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .check(now, authenticated, is_expensive)
    }

    pub fn check_forum_unreads_rate_limit(&self) -> bool {
        let now = now_millis();
        self.rate_limiter
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .check_forum_unreads(now)
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
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        st.guilds.remove(guild_id);
        st.active_guilds.remove(guild_id);
        st.thread_subscriptions.remove(guild_id);
        st.thread_member_lists.remove(guild_id);
    }

    pub fn is_bot(&self) -> bool {
        self.state.read().unwrap_or_else(|e| e.into_inner()).is_bot
    }

    pub fn has_channel_threads(&self) -> bool {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        st.is_bot
            || SessionFlags::from_bits_truncate(st.flags).contains(SessionFlags::CHANNEL_THREADS)
    }

    pub fn set_guild_active(&self, guild_id: &str, active: bool) {
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        if active {
            st.active_guilds.insert(guild_id.to_string());
        } else {
            st.active_guilds.remove(guild_id);
            st.thread_subscriptions.remove(guild_id);
        }
    }

    pub fn set_thread_subscription(&self, guild_id: &str, enabled: bool) {
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        if enabled {
            st.thread_subscriptions.insert(guild_id.to_string());
        } else {
            st.thread_subscriptions.remove(guild_id);
        }
    }

    pub fn set_thread_member_lists(&self, guild_id: &str, thread_ids: Vec<String>) -> Vec<String> {
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        let prev: HashSet<String> = st
            .thread_member_lists
            .get(guild_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        let newly_added: Vec<String> = thread_ids
            .iter()
            .filter(|id| !prev.contains(id.as_str()))
            .cloned()
            .collect();
        if thread_ids.is_empty() {
            st.thread_member_lists.remove(guild_id);
        } else {
            st.thread_member_lists
                .insert(guild_id.to_string(), thread_ids);
        }
        newly_added
    }

    pub fn is_subscribed_to_thread_members(&self, guild_id: &str, thread_id: &str) -> bool {
        let st = self.state.read().unwrap_or_else(|e| e.into_inner());
        st.thread_member_lists
            .get(guild_id)
            .is_some_and(|ids| ids.iter().any(|id| id == thread_id))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn set_authenticated(
        &self,
        user_id: String,
        auth_session_id: String,
        is_bot: bool,
        flags: u32,
        status: PresenceStatus,
        afk: bool,
        mobile: bool,
        guilds: HashSet<String>,
    ) {
        let mut st = self.state.write().unwrap_or_else(|e| e.into_inner());
        st.user_id = Some(user_id);
        st.auth_session_id = Some(auth_session_id);
        st.is_bot = is_bot;
        st.flags = flags;
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
        self.state.write().unwrap_or_else(|e| e.into_inner()).status = status;
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

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
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
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_rate_limiter_enforces_pre_auth_expensive_and_window_reset() {
        let mut rl = GatewayRateLimiter::new(1_000);

        for _ in 0..GATEWAY_PRE_AUTH_MAX_EVENTS {
            assert!(rl.check(1_000, false, false));
        }
        assert!(!rl.check(1_000, false, false));

        for _ in 0..GATEWAY_MAX_EXPENSIVE_OPS_PER_WINDOW {
            assert!(rl.check(1_000, true, true));
        }
        assert!(!rl.check(1_000, true, true));

        let next_window = 1_000 + GATEWAY_RATE_LIMIT_WINDOW_SECS * 1000;
        assert!(rl.check(next_window, true, true));
    }
}
