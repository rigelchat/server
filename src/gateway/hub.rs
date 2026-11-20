use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::{HEARTBEAT_INTERVAL, close_codes};
use crate::gateway::handlers::presence::{combined_status, public_status};
use crate::gateway::session::GatewaySession;
use crate::models::user::{CustomStatus, PartialUser};

const HEARTBEAT_TIMEOUT_MS: u64 = (HEARTBEAT_INTERVAL as u64) * 2 + 5_000;
const REAPER_INTERVAL_SECS: u64 = 15;

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn aggregate_live_state<'a>(
    items: impl Iterator<Item = (&'a str, bool, bool)>,
) -> (&'static str, bool, bool) {
    let mut statuses = Vec::new();
    let mut all_afk = true;
    let mut any_mobile = false;

    for (status, afk, mobile) in items {
        statuses.push(status);
        all_afk &= afk;
        any_mobile |= mobile;
    }

    if statuses.is_empty() {
        return ("offline", false, false);
    }

    let pub_st = public_status(combined_status(statuses));
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

    #[allow(clippy::too_many_arguments)]
    pub fn authenticate(
        &self,
        session_id: &str,
        dialect: ClientDialect,
        user_id: String,
        auth_session_id: String,
        status: String,
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
            dialect,
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

        let prev_live_state = {
            let mut snapshots = Vec::new();
            if let Some(sids) = inner.user_sessions.get(&uid) {
                for sid in sids {
                    if sid == session_id {
                        snapshots.push(removed.snapshot_presence());
                    } else if let Some(s) = inner.sessions.get(sid) {
                        snapshots.push(s.snapshot_presence());
                    }
                }
            } else {
                snapshots.push(removed.snapshot_presence());
            }
            aggregate_live_state(
                snapshots
                    .iter()
                    .map(|(st, afk, mob)| (st.as_str(), *afk, *mob)),
            )
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

    pub fn online_user_ids(&self) -> HashSet<String> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let mut out = HashSet::new();

        for (uid, sids) in &inner.user_sessions {
            let snapshots: Vec<_> = sids
                .iter()
                .filter_map(|sid| inner.sessions.get(sid))
                .map(|s| s.snapshot_presence())
                .collect();
            let (pub_st, _, _) = aggregate_live_state(
                snapshots
                    .iter()
                    .map(|(st, afk, mob)| (st.as_str(), *afk, *mob)),
            );
            if pub_st != "offline" {
                out.insert(uid.clone());
            }
        }

        out
    }

    pub fn count_guild_online(&self, guild_id: &str) -> u64 {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
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
            .filter(|uid| {
                let Some(usids) = inner.user_sessions.get(uid) else {
                    return false;
                };
                let snapshots: Vec<_> = usids
                    .iter()
                    .filter_map(|sid| inner.sessions.get(sid))
                    .map(|s| s.snapshot_presence())
                    .collect();
                let (pub_st, _, _) = aggregate_live_state(
                    snapshots
                        .iter()
                        .map(|(st, afk, mob)| (st.as_str(), *afk, *mob)),
                );
                pub_st != "offline"
            })
            .count() as u64
    }

    pub fn collect_user_live_state(&self, user_id: &str) -> (&'static str, bool, bool) {
        let sessions = self.sessions_for_user(user_id);
        let snapshots: Vec<_> = sessions.iter().map(|s| s.snapshot_presence()).collect();
        aggregate_live_state(
            snapshots
                .iter()
                .map(|(st, afk, mob)| (st.as_str(), *afk, *mob)),
        )
    }

    pub fn guild_live_states(
        &self,
        guild_id: &str,
    ) -> HashMap<String, (&'static str, bool, bool)> {
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
            let Some(usids) = inner.user_sessions.get(&uid) else {
                continue;
            };
            let snapshots: Vec<_> = usids
                .iter()
                .filter_map(|sid| inner.sessions.get(sid))
                .map(|s| s.snapshot_presence())
                .collect();
            let state = aggregate_live_state(
                snapshots
                    .iter()
                    .map(|(st, afk, mob)| (st.as_str(), *afk, *mob)),
            );
            out.insert(uid, state);
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
            let Some(usids) = inner.user_sessions.get(&uid) else {
                continue;
            };
            let snapshots: Vec<_> = usids
                .iter()
                .filter_map(|sid| inner.sessions.get(sid))
                .map(|s| s.snapshot_presence())
                .collect();
            let (pub_st, afk, mobile) = aggregate_live_state(
                snapshots
                    .iter()
                    .map(|(st, afk, mob)| (st.as_str(), *afk, *mob)),
            );
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
            let (pub_st, afk, mobile) = if let Some(usids) = inner.user_sessions.get(uid) {
                let snapshots: Vec<_> = usids
                    .iter()
                    .filter_map(|sid| inner.sessions.get(sid))
                    .map(|s| s.snapshot_presence())
                    .collect();
                aggregate_live_state(
                    snapshots
                        .iter()
                        .map(|(st, afk, mob)| (st.as_str(), *afk, *mob)),
                )
            } else {
                ("offline", false, false)
            };

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
    ) -> Vec<(Arc<GatewaySession>, ClientDialect, String, [i64; 2])> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some(sids) = inner.guild_sessions.get(guild_id) else {
            return Vec::new();
        };

        sids.iter()
            .filter_map(|sid| inner.sessions.get(sid))
            .filter(|s| s.is_authenticated())
            .filter_map(|s| {
                s.member_list_subscription(guild_id)
                    .map(|(cid, range)| (Arc::clone(s), s.dialect(), cid, range))
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
        for session in self.sessions_for_user(user_id) {
            session.set_status(status.to_string());
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