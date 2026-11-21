use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::constants::gateway::{
    HEARTBEAT_TIMEOUT_MS, IDENTIFY_TIMEOUT_MS, REAPER_INTERVAL_SECS, close_codes,
};
use crate::constants::user::PresenceStatus;
use crate::gateway::session::GatewaySession;
use crate::models::user::{CustomStatus, PartialUser};



pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn aggregate_live_state(
    items: impl Iterator<Item = (PresenceStatus, bool, bool)>,
) -> (&'static str, bool, bool) {
    let mut any = false;
    let mut has_dnd = false;
    let mut has_online = false;
    let mut has_idle = false;
    let mut all_afk = true;
    let mut any_mobile = false;

    for (status, afk, mobile) in items {
        any = true;
        match status {
            PresenceStatus::Dnd => has_dnd = true,
            PresenceStatus::Online => has_online = true,
            PresenceStatus::Idle => has_idle = true,
            PresenceStatus::Invisible | PresenceStatus::Offline => {}
        }
        all_afk &= afk;
        any_mobile |= mobile;
    }

    if !any {
        return ("offline", false, false);
    }

    let pub_st = if has_dnd {
        "dnd"
    } else if has_online {
        "online"
    } else if has_idle {
        "idle"
    } else {
        "offline"
    };

    if pub_st == "offline" {
        ("offline", false, false)
    } else {
        (pub_st, all_afk && !any_mobile, any_mobile)
    }
}

pub struct LivePresenceSnapshot {
    pub user_id: String,
    pub status: &'static str,
    pub mobile: bool,
    pub afk: bool,
    pub custom_status: Option<CustomStatus>,
    pub partial_user: Option<PartialUser>,
}

pub struct UnregisteredSession {
    pub user_id: String,
    pub guilds: HashSet<String>,
    pub has_other_session: bool,
    pub prev_live_state: (&'static str, bool, bool),
    pub partial_user: Option<PartialUser>,
    pub custom_status: Option<CustomStatus>,
}

#[derive(Default)]
struct HubInner {
    sessions: HashMap<String, Arc<GatewaySession>>,
    user_sessions: HashMap<String, HashSet<String>>,
    guild_sessions: HashMap<String, HashSet<String>>,
    user_profiles: HashMap<String, PartialUser>,
    user_custom_statuses: HashMap<String, Option<CustomStatus>>,
}

#[derive(Clone, Default)]
pub struct GatewayHub {
    inner: Arc<RwLock<HubInner>>,
}

impl GatewayHub {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&self, session: Arc<GatewaySession>) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.sessions.insert(session.id.clone(), session);
    }

    pub fn get(&self, session_id: &str) -> Option<Arc<GatewaySession>> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.sessions.get(session_id).cloned()
    }

    pub fn close_session(&self, session_id: &str, close: (u16, &'static str)) {
        if let Some(session) = self.get(session_id) {
            session.close(close);
        }
    }

    pub fn close_auth_session(&self, user_id: &str, auth_session_id: &str) {
        for session in self.sessions_for_user(user_id) {
            if session.auth_session_id().as_deref() == Some(auth_session_id) {
                session.close(close_codes::AUTHENTICATION_FAILED);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn authenticate(
        &self,
        session_id: &str,
        user_id: String,
        auth_session_id: String,
        status: PresenceStatus,
        afk: bool,
        mobile: bool,
        guild_ids: &[String],
        partial_user: PartialUser,
        custom_status: Option<CustomStatus>,
    ) -> Option<Arc<GatewaySession>> {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let session = inner.sessions.get(session_id)?.clone();

        let guild_set: HashSet<String> = guild_ids.iter().cloned().collect();
        session.set_authenticated(
            user_id.clone(),
            auth_session_id,
            status,
            afk,
            mobile,
            guild_set,
        );
        session.touch_heartbeat();

        inner
            .user_sessions
            .entry(user_id.clone())
            .or_default()
            .insert(session_id.to_string());

        for gid in guild_ids {
            inner
                .guild_sessions
                .entry(gid.clone())
                .or_default()
                .insert(session_id.to_string());
        }

        inner.user_profiles.insert(user_id.clone(), partial_user);
        inner.user_custom_statuses.insert(user_id, custom_status);

        Some(session)
    }

    pub fn unregister(&self, session_id: &str) -> (Option<UnregisteredSession>, usize) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let Some(removed) = inner.sessions.remove(session_id) else {
            return (None, inner.sessions.len());
        };

        let Some(uid) = removed.user_id() else {
            return (None, inner.sessions.len());
        };

        let guilds = removed.guilds();

        let prev_live_state = if let Some(sids) = inner.user_sessions.get(&uid) {
            aggregate_live_state(sids.iter().filter_map(|sid| {
                if sid == session_id {
                    Some(removed.snapshot_presence())
                } else {
                    inner.sessions.get(sid).map(|s| s.snapshot_presence())
                }
            }))
        } else {
            aggregate_live_state(std::iter::once(removed.snapshot_presence()))
        };

        if let Some(sids) = inner.user_sessions.get_mut(&uid) {
            sids.remove(session_id);
            if sids.is_empty() {
                inner.user_sessions.remove(&uid);
            }
        }

        for gid in &guilds {
            if let Some(sids) = inner.guild_sessions.get_mut(gid) {
                sids.remove(session_id);
                if sids.is_empty() {
                    inner.guild_sessions.remove(gid);
                }
            }
        }

        let has_other_session = inner.user_sessions.contains_key(&uid);
        let (partial_user, custom_status) = if has_other_session {
            (
                inner.user_profiles.get(&uid).cloned(),
                inner.user_custom_statuses.get(&uid).cloned().flatten(),
            )
        } else {
            (
                inner.user_profiles.remove(&uid),
                inner.user_custom_statuses.remove(&uid).flatten(),
            )
        };

        (
            Some(UnregisteredSession {
                user_id: uid,
                guilds,
                has_other_session,
                prev_live_state,
                partial_user,
                custom_status,
            }),
            inner.sessions.len(),
        )
    }

    pub fn sessions_for_user(&self, user_id: &str) -> Vec<Arc<GatewaySession>> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some(sids) = inner.user_sessions.get(user_id) else {
            return Vec::new();
        };
        sids.iter()
            .filter_map(|sid| inner.sessions.get(sid).cloned())
            .collect()
    }

    pub fn sessions_for_guild(&self, guild_id: &str) -> Vec<Arc<GatewaySession>> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some(sids) = inner.guild_sessions.get(guild_id) else {
            return Vec::new();
        };
        sids.iter()
            .filter_map(|sid| inner.sessions.get(sid).cloned())
            .collect()
    }

    pub fn sessions_for_guilds_except(
        &self,
        guild_ids: &HashSet<String>,
        exclude_user_id: &str,
    ) -> Vec<(Arc<GatewaySession>, Vec<String>)> {
        if guild_ids.is_empty() {
            return Vec::new();
        }
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut by_session: HashMap<String, (Arc<GatewaySession>, Vec<String>)> = HashMap::new();

        for gid in guild_ids {
            let Some(sids) = inner.guild_sessions.get(gid) else {
                continue;
            };
            for sid in sids {
                let Some(session) = inner.sessions.get(sid) else {
                    continue;
                };
                if session.user_id().as_deref() == Some(exclude_user_id) {
                    continue;
                }
                by_session
                    .entry(sid.clone())
                    .or_insert_with(|| (Arc::clone(session), Vec::new()))
                    .1
                    .push(gid.clone());
            }
        }

        by_session.into_values().collect()
    }

    pub fn add_user_guild(&self, user_id: &str, guild_id: &str) {
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let inner = &mut *guard;
        let Some(sids) = inner.user_sessions.get(user_id).cloned() else {
            return;
        };
        let entry = inner
            .guild_sessions
            .entry(guild_id.to_string())
            .or_default();
        for sid in sids {
            if let Some(session) = inner.sessions.get(&sid) {
                session.add_guild(guild_id);
                entry.insert(sid);
            }
        }
    }

    pub fn remove_user_guild(&self, user_id: &str, guild_id: &str) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let Some(sids) = inner.user_sessions.get(user_id).cloned() else {
            return;
        };
        for sid in &sids {
            if let Some(session) = inner.sessions.get(sid) {
                session.remove_guild(guild_id);
                session.remove_member_list_subscription(guild_id);
            }
        }
        if let Some(guild_set) = inner.guild_sessions.get_mut(guild_id) {
            for sid in &sids {
                guild_set.remove(sid);
            }
            if guild_set.is_empty() {
                inner.guild_sessions.remove(guild_id);
            }
        }
    }

    fn user_live_state_locked(
        inner: &HubInner,
        user_id: &str,
    ) -> (&'static str, bool, bool) {
        let Some(usids) = inner.user_sessions.get(user_id) else {
            return ("offline", false, false);
        };
        aggregate_live_state(
            usids
                .iter()
                .filter_map(|sid| inner.sessions.get(sid))
                .map(|s| s.snapshot_presence()),
        )
    }

    pub fn count_guild_online(&self, guild_id: &str) -> u64 {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        Self::count_guild_online_locked(&inner, guild_id)
    }

    pub fn count_guilds_online(&self, guild_ids: &[String]) -> HashMap<String, u64> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut out = HashMap::with_capacity(guild_ids.len());
        for gid in guild_ids {
            out.insert(gid.clone(), Self::count_guild_online_locked(&inner, gid));
        }
        out
    }

    fn count_guild_online_locked(inner: &HubInner, guild_id: &str) -> u64 {
        let Some(sids) = inner.guild_sessions.get(guild_id) else {
            return 0;
        };

        let mut user_ids = HashSet::new();
        for sid in sids {
            if let Some(session) = inner.sessions.get(sid) {
                if let Some(uid) = session.user_id() {
                    user_ids.insert(uid);
                }
            }
        }

        user_ids
            .into_iter()
            .filter(|uid| Self::user_live_state_locked(inner, uid).0 != "offline")
            .count() as u64
    }

    pub fn collect_user_live_state(&self, user_id: &str) -> (&'static str, bool, bool) {
        let sessions = self.sessions_for_user(user_id);
        aggregate_live_state(sessions.iter().map(|s| s.snapshot_presence()))
    }

    pub fn guild_live_presences(
        &self,
        guild_id: &str,
    ) -> HashMap<String, (&'static str, bool, bool, Option<CustomStatus>)> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some(sids) = inner.guild_sessions.get(guild_id) else {
            return HashMap::new();
        };

        let mut user_ids = HashSet::new();
        for sid in sids {
            if let Some(session) = inner.sessions.get(sid) {
                if let Some(uid) = session.user_id() {
                    user_ids.insert(uid);
                }
            }
        }

        let mut out = HashMap::with_capacity(user_ids.len());
        for uid in user_ids {
            let (status, afk, mobile) = Self::user_live_state_locked(&inner, &uid);
            let custom_status = if status == "offline" {
                None
            } else {
                inner.user_custom_statuses.get(&uid).cloned().flatten()
            };
            out.insert(uid, (status, afk, mobile, custom_status));
        }
        out
    }

    pub fn ready_presences_for_guilds(
        &self,
        viewer_id: &str,
        guild_ids: &[String],
    ) -> Vec<LivePresenceSnapshot> {
        if guild_ids.is_empty() {
            return Vec::new();
        }
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut peer_ids = HashSet::new();

        for gid in guild_ids {
            if let Some(sids) = inner.guild_sessions.get(gid) {
                for sid in sids {
                    if let Some(session) = inner.sessions.get(sid) {
                        if let Some(uid) = session.user_id() {
                            if uid != viewer_id {
                                peer_ids.insert(uid);
                            }
                        }
                    }
                }
            }
        }

        let mut out = Vec::with_capacity(peer_ids.len());
        for uid in peer_ids {
            let (pub_st, afk, mobile) = Self::user_live_state_locked(&inner, &uid);
            if pub_st == "offline" {
                continue;
            }
            out.push(LivePresenceSnapshot {
                custom_status: inner.user_custom_statuses.get(&uid).cloned().flatten(),
                partial_user: inner.user_profiles.get(&uid).cloned(),
                user_id: uid,
                status: pub_st,
                mobile,
                afk,
            });
        }
        out
    }

    pub fn presence_snapshots(&self, user_ids: &[String]) -> Vec<LivePresenceSnapshot> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::with_capacity(user_ids.len());

        for uid in user_ids {
            let (pub_st, afk, mobile) = Self::user_live_state_locked(&inner, uid);
            let custom_status = if pub_st == "offline" {
                None
            } else {
                inner.user_custom_statuses.get(uid).cloned().flatten()
            };

            out.push(LivePresenceSnapshot {
                user_id: uid.clone(),
                status: pub_st,
                mobile,
                afk,
                custom_status,
                partial_user: inner.user_profiles.get(uid).cloned(),
            });
        }

        out
    }

    pub fn guild_member_list_subscribers(
        &self,
        guild_id: &str,
    ) -> Vec<(Arc<GatewaySession>, String, [i64; 2])> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some(sids) = inner.guild_sessions.get(guild_id) else {
            return Vec::new();
        };

        sids.iter()
            .filter_map(|sid| inner.sessions.get(sid))
            .filter(|s| s.is_authenticated())
            .filter_map(|s| {
                s.member_list_subscription(guild_id)
                    .map(|(cid, range)| (Arc::clone(s), cid, range))
            })
            .collect()
    }

    pub fn update_partial_user(&self, user: &PartialUser) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if inner.user_sessions.contains_key(&user.id) {
            inner.user_profiles.insert(user.id.clone(), user.clone());
        }
    }

    pub fn get_cached_partial_user(&self, user_id: &str) -> Option<PartialUser> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.user_profiles.get(user_id).cloned()
    }

    pub fn set_custom_status(&self, user_id: &str, custom_status: Option<CustomStatus>) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if inner.user_sessions.contains_key(user_id) {
            inner
                .user_custom_statuses
                .insert(user_id.to_string(), custom_status);
        }
    }

    pub fn get_custom_status(&self, user_id: &str) -> Option<CustomStatus> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner
            .user_custom_statuses
            .get(user_id)
            .cloned()
            .flatten()
    }

    pub fn set_user_status(&self, user_id: &str, status: &str) {
        let Some(parsed) = PresenceStatus::parse(status) else {
            return;
        };
        for session in self.sessions_for_user(user_id) {
            session.set_status(parsed);
        }
    }

    pub fn spawn_heartbeat_reaper(&self) {
        let hub = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_secs(REAPER_INTERVAL_SECS));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

            loop {
                ticker.tick().await;
                let now = now_millis();
                let expired: Vec<Arc<GatewaySession>> = {
                    let inner = hub.inner.read().unwrap_or_else(|e| e.into_inner());
                    inner
                        .sessions
                        .values()
                        .filter(|s| {
                            now.saturating_sub(s.last_heartbeat_ms()) > HEARTBEAT_TIMEOUT_MS
                                || (!s.is_authenticated()
                                    && now.saturating_sub(s.connected_at_ms()) > IDENTIFY_TIMEOUT_MS)
                        })
                        .cloned()
                        .collect()
                };

                for session in expired {
                    tracing::debug!(session_id = %session.id, "Closing timed-out gateway session");
                    session.close(close_codes::SESSION_TIMED_OUT);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    fn test_user(id: &str) -> PartialUser {
        PartialUser {
            id: id.to_string(),
            username: format!("user_{id}"),
            discriminator: "0000".to_string(),
            global_name: None,
            avatar: None,
            avatar_color: None,
            bot: false,
            flags: 0,
        }
    }

    #[test]
    fn aggregate_live_state_precedence_and_invisible_masking() {
        assert_eq!(
            aggregate_live_state(std::iter::empty()),
            ("offline", false, false)
        );

        let inv = [
            (PresenceStatus::Invisible, false, true),
            (PresenceStatus::Invisible, true, false),
        ];
        assert_eq!(
            aggregate_live_state(inv.iter().copied()),
            ("offline", false, false)
        );

        let mixed = [
            (PresenceStatus::Online, true, false),
            (PresenceStatus::Dnd, false, true),
        ];
        assert_eq!(
            aggregate_live_state(mixed.iter().copied()),
            ("dnd", false, true)
        );

        let idle_afk = [
            (PresenceStatus::Idle, true, false),
            (PresenceStatus::Invisible, true, false),
        ];
        assert_eq!(
            aggregate_live_state(idle_afk.iter().copied()),
            ("idle", true, false)
        );
    }

    #[test]
    fn hub_ready_presences_excludes_viewer_and_invisible() {
        let hub = GatewayHub::new();
        let (tx1, _rx1) = mpsc::channel(8);
        let (tx2, _rx2) = mpsc::channel(8);
        let (tx3, _rx3) = mpsc::channel(8);

        hub.register(Arc::new(GatewaySession::new("s1".into(), tx1)));
        hub.register(Arc::new(GatewaySession::new("s2".into(), tx2)));
        hub.register(Arc::new(GatewaySession::new("s3".into(), tx3)));

        let guilds = vec!["g1".to_string()];
        hub.authenticate("s1", "u1".into(), "a1".into(), PresenceStatus::Online, false, false, &guilds, test_user("u1"), None);
        hub.authenticate("s2", "u2".into(), "a2".into(), PresenceStatus::Dnd, false, true, &guilds, test_user("u2"), None);
        hub.authenticate("s3", "u3".into(), "a3".into(), PresenceStatus::Invisible, false, false, &guilds, test_user("u3"), None);

        let presences = hub.ready_presences_for_guilds("u1", &guilds);
        assert_eq!(presences.len(), 1);
        assert_eq!(presences[0].user_id, "u2");
        assert_eq!(presences[0].status, "dnd");
        assert!(presences[0].mobile);
        assert_eq!(hub.count_guild_online("g1"), 2);

        let (unreg, remaining) = hub.unregister("s2");
        assert_eq!(remaining, 2);
        let unreg = unreg.unwrap();
        assert!(!unreg.has_other_session);
        assert_eq!(unreg.prev_live_state, ("dnd", false, true));
        assert_eq!(hub.count_guild_online("g1"), 1);
    }
}
