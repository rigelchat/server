use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::json;
use tracing::error;

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::events;
use crate::constants::user::VALID_STATUSES;
use crate::models::gateway::ready::ClientSessionInfo;
use crate::models::rest::users::CustomStatusInput;
use crate::models::serde_helpers::deserialize_double_option;
use crate::models::user::{CustomStatus, UserSettingsExtras};
use crate::services::user_service;

#[derive(Debug, Deserialize)]
pub struct PresenceUpdatePayload {
    pub status: String,
    pub afk: Option<bool>,
    pub mobile: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub custom_status: Option<Option<CustomStatusInput>>,
}

pub fn combined_status<'a>(statuses: impl IntoIterator<Item = &'a str>) -> &'static str {
    let mut has_online = false;
    let mut has_idle = false;
    let mut has_invisible = false;

    for s in statuses {
        match s {
            "dnd" => return "dnd",
            "online" => has_online = true,
            "idle" => has_idle = true,
            "invisible" => has_invisible = true,
            _ => {}
        }
    }

    if has_online {
        "online"
    } else if has_idle {
        "idle"
    } else if has_invisible {
        "invisible"
    } else {
        "offline"
    }
}

pub fn public_status(combined: &str) -> &'static str {
    match combined {
        "dnd" => "dnd",
        "idle" => "idle",
        "online" => "online",
        _ => "offline",
    }
}

pub async fn build_user_sessions(state: &AppState, user_id: &str) -> Vec<ClientSessionInfo> {
    let sessions = state.sessions.read().await;
    let live: Vec<ClientSessionInfo> = sessions
        .values()
        .filter(|s| s.user_id.as_deref() == Some(user_id))
        .map(|s| ClientSessionInfo {
            session_id: s.id.clone(),
            status: s.status.clone(),
            afk: s.afk,
            mobile: s.mobile,
        })
        .collect();

    let combined = combined_status(live.iter().map(|s| s.status.as_str()));
    let all_afk = !live.is_empty() && live.iter().all(|s| s.afk);
    let any_mobile = live.iter().any(|s| s.mobile);

    let mut out = Vec::with_capacity(live.len() + 1);
    out.push(ClientSessionInfo {
        session_id: "all".to_string(),
        status: combined.to_string(),
        afk: all_afk,
        mobile: any_mobile,
    });
    out.extend(live);
    out
}

pub async fn dispatch_sessions_replace(state: &AppState, user_id: &str) {
    let list = build_user_sessions(state, user_id).await;
    state
        .dispatch_user(events::SESSIONS_REPLACE, &list, user_id)
        .await;
}

pub async fn collect_user_live_state(state: &AppState, user_id: &str) -> (&'static str, bool, bool) {
    let sessions = state.sessions.read().await;
    let mut statuses: Vec<&str> = Vec::new();
    let mut all_afk = true;
    let mut any_mobile = false;

    for s in sessions.values() {
        if s.user_id.as_deref() == Some(user_id) {
            statuses.push(s.status.as_str());
            all_afk &= s.afk;
            any_mobile |= s.mobile;
        }
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

pub async fn user_public_status(state: &AppState, user_id: &str) -> &'static str {
    collect_user_live_state(state, user_id).await.0
}

pub async fn fetch_user_custom_status(state: &AppState, user_id: &str) -> Option<CustomStatus> {
    let extra: Option<String> =
        sqlx::query_scalar("SELECT extra_settings FROM user_settings WHERE id = ?")
            .bind(user_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .flatten();

    extra
        .as_deref()
        .and_then(|s| serde_json::from_str::<UserSettingsExtras>(s).ok())
        .and_then(|e| e.custom_status)
}

pub async fn build_presence_payload(
    state: &AppState,
    user_id: &str,
    guild_id: Option<&str>,
    dialect: ClientDialect,
) -> serde_json::Value {
    let (pub_st, afk, mobile) = collect_user_live_state(state, user_id).await;
    let custom_status = if pub_st == "offline" {
        None
    } else {
        fetch_user_custom_status(state, user_id).await
    };

    match dialect {
        ClientDialect::Fluxer => {
            let user_val = user_service::get_partial_user(state, user_id)
                .await
                .ok()
                .and_then(|u| serde_json::to_value(u).ok())
                .unwrap_or_else(|| json!({ "id": user_id }));
            let mut obj = json!({
                "user": user_val,
                "status": pub_st,
                "mobile": mobile,
                "afk": afk,
                "custom_status": custom_status,
            });
            if let Some(gid) = guild_id {
                obj["guild_id"] = json!(gid);
            }
            obj
        }
        ClientDialect::Discord => {
            let client_status = if pub_st == "offline" {
                json!({})
            } else {
                json!({ "desktop": pub_st })
            };
            let mut obj = json!({
                "user": { "id": user_id },
                "status": pub_st,
                "activities": [],
                "client_status": client_status,
            });
            if let Some(gid) = guild_id {
                obj["guild_id"] = json!(gid);
            }
            obj
        }
    }
}

pub async fn build_ready_presences(
    state: &AppState,
    viewer_id: &str,
    guild_ids: &[String],
) -> Vec<serde_json::Value> {
    if guild_ids.is_empty() {
        return vec![];
    }

    let viewer_guilds: HashSet<&str> = guild_ids.iter().map(String::as_str).collect();
    let peer_ids: Vec<String> = {
        let sessions = state.sessions.read().await;
        let mut seen: HashMap<String, Vec<(&str, bool, bool)>> = HashMap::new();
        for s in sessions.values() {
            let Some(uid) = &s.user_id else {
                continue;
            };
            if uid == viewer_id {
                continue;
            }
            if !s.guilds.iter().any(|g| viewer_guilds.contains(g.as_str())) {
                continue;
            }
            seen.entry(uid.clone()).or_default();
        }
        seen.into_keys().collect()
    };

    let mut out = Vec::with_capacity(peer_ids.len());
    for uid in peer_ids {
        let (pub_st, _, _) = collect_user_live_state(state, &uid).await;
        if pub_st != "offline" {
            out.push(build_presence_payload(state, &uid, None, ClientDialect::Fluxer).await);
        }
    }
    out
}

pub async fn handle(session_id: &str, state: &AppState, payload: PresenceUpdatePayload) {
    if !VALID_STATUSES.contains(&payload.status.as_str()) && payload.status != "offline" {
        return;
    }

    let (user_id, guild_ids) = {
        let mut sessions = state.sessions.write().await;
        let Some(session) = sessions.get_mut(session_id) else {
            return;
        };
        let Some(uid) = session.user_id.clone() else {
            return;
        };

        session.status = if payload.status == "offline" {
            "offline".to_string()
        } else {
            payload.status.clone()
        };
        if let Some(afk) = payload.afk {
            session.afk = afk;
        }
        if let Some(mobile) = payload.mobile {
            session.mobile = mobile;
        }

        (uid, session.guilds.clone())
    };

    if payload.status != "offline" {
        if let Err(err) = sqlx::query("UPDATE user_settings SET status = ? WHERE id = ?")
            .bind(&payload.status)
            .bind(&user_id)
            .execute(&state.db)
            .await
        {
            error!(error = %err, "Failed to update status in user_settings");
        }
    }

    let existing_extra: Option<String> =
        sqlx::query_scalar("SELECT extra_settings FROM user_settings WHERE id = ?")
            .bind(&user_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .flatten();

    let mut extras: UserSettingsExtras = existing_extra
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    if let Some(cs_opt) = payload.custom_status {
        extras.custom_status = cs_opt.map(CustomStatus::from);
        if let Ok(json_str) = serde_json::to_string(&extras) {
            if let Err(err) = sqlx::query("UPDATE user_settings SET extra_settings = ? WHERE id = ?")
                .bind(&json_str)
                .bind(&user_id)
                .execute(&state.db)
                .await
            {
                error!(error = %err, "Failed to update custom_status in user_settings");
            }
        }
    }

    dispatch_sessions_replace(state, &user_id).await;

    let pub_status = user_public_status(state, &user_id).await;
    broadcast_presence(
        state,
        &user_id,
        &guild_ids,
        pub_status,
        extras.custom_status.as_ref(),
    )
    .await;

    for gid in &guild_ids {
        super::lazy_request::refresh_guild_member_lists(state, gid).await;
    }
}

pub async fn broadcast_presence(
    state: &AppState,
    user_id: &str,
    guild_ids: &HashSet<String>,
    public_status: &str,
    custom_status: Option<&CustomStatus>,
) {
    if guild_ids.is_empty() {
        return;
    }

    let partial_user = user_service::get_partial_user(state, user_id).await.ok();
    let (_, afk, mobile) = collect_user_live_state(state, user_id).await;
    let custom_status = if public_status == "offline" {
        None
    } else {
        custom_status
    };

    let sessions = state.sessions.read().await;
    for peer in sessions.values() {
        let Some(peer_uid) = &peer.user_id else {
            continue;
        };
        if peer_uid == user_id {
            continue;
        }
        for guild_id in peer.guilds.intersection(guild_ids) {
            let payload = match peer.dialect {
                ClientDialect::Fluxer => {
                    let user_val = partial_user
                        .as_ref()
                        .and_then(|u| serde_json::to_value(u).ok())
                        .unwrap_or_else(|| json!({ "id": user_id }));
                    json!({
                        "guild_id": guild_id,
                        "user": user_val,
                        "status": public_status,
                        "mobile": mobile,
                        "afk": afk,
                        "custom_status": custom_status,
                    })
                }
                ClientDialect::Discord => {
                    let client_status = if public_status == "offline" {
                        json!({})
                    } else {
                        json!({ "desktop": public_status })
                    };
                    json!({
                        "guild_id": guild_id,
                        "user": { "id": user_id },
                        "status": public_status,
                        "activities": [],
                        "client_status": client_status,
                    })
                }
            };
            peer.dispatch(events::PRESENCE_UPDATE, payload).await;
        }
    }
}