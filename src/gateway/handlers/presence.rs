use std::collections::{HashMap, HashSet};

use serde::Deserialize;
use serde_json::json;
use sqlx::{MySql, QueryBuilder};
use tracing::error;

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::events;
use crate::constants::user::VALID_STATUSES;
use crate::gateway::hub::LivePresenceSnapshot;
use crate::models::gateway::ready::ClientSessionInfo;
use crate::models::rest::users::CustomStatusInput;
use crate::models::serde_helpers::deserialize_double_option;
use crate::models::user::{CustomStatus, PartialUser, UserRow, UserSettingsExtras};
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
    let live: Vec<ClientSessionInfo> = state
        .hub
        .sessions_for_user(user_id)
        .into_iter()
        .map(|s| s.client_session_info())
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
    state.hub.collect_user_live_state(user_id)
}

pub async fn user_public_status(state: &AppState, user_id: &str) -> &'static str {
    state.hub.collect_user_live_state(user_id).0
}

async fn fetch_partial_users_batched(
    state: &AppState,
    user_ids: &[String],
) -> HashMap<String, PartialUser> {
    if user_ids.is_empty() {
        return HashMap::new();
    }

    let mut qb = QueryBuilder::<MySql>::new("SELECT * FROM users WHERE id IN (");
    let mut sep = qb.separated(", ");
    for uid in user_ids {
        sep.push_bind(uid);
    }
    qb.push(")");

    qb.build_query_as::<UserRow>()
        .fetch_all(&state.db)
        .await
        .unwrap_or_default()
        .iter()
        .map(|row| (row.id.clone(), PartialUser::from(row)))
        .collect()
}

fn format_presence_json(
    snap: &LivePresenceSnapshot,
    fetched_users: &HashMap<String, PartialUser>,
    guild_id: Option<&str>,
    dialect: ClientDialect,
) -> serde_json::Value {
    match dialect {
        ClientDialect::Fluxer => {
            let user_val = snap
                .partial_user
                .as_ref()
                .or_else(|| fetched_users.get(&snap.user_id))
                .and_then(|u| serde_json::to_value(u).ok())
                .unwrap_or_else(|| json!({ "id": snap.user_id }));
            let mut obj = json!({
                "user": user_val,
                "status": snap.status,
                "mobile": snap.mobile,
                "afk": snap.afk,
                "custom_status": snap.custom_status,
            });
            if let Some(gid) = guild_id {
                obj["guild_id"] = json!(gid);
            }
            obj
        }
        ClientDialect::Discord => {
            let client_status = if snap.status == "offline" {
                json!({})
            } else {
                json!({ "desktop": snap.status })
            };
            let mut obj = json!({
                "user": { "id": snap.user_id },
                "status": snap.status,
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
    let snapshots = state.hub.ready_presences_for_guilds(viewer_id, guild_ids);
    if snapshots.is_empty() {
        return vec![];
    }

    let missing_ids: Vec<String> = snapshots
        .iter()
        .filter(|s| s.partial_user.is_none())
        .map(|s| s.user_id.clone())
        .collect();
    let fetched = fetch_partial_users_batched(state, &missing_ids).await;

    snapshots
        .iter()
        .map(|snap| format_presence_json(snap, &fetched, None, ClientDialect::Fluxer))
        .collect()
}

pub async fn build_presence_payloads(
    state: &AppState,
    user_ids: &[String],
    guild_id: Option<&str>,
    dialect: ClientDialect,
) -> Vec<serde_json::Value> {
    if user_ids.is_empty() {
        return vec![];
    }

    let snapshots = state.hub.presence_snapshots(user_ids);
    let fetched = if dialect == ClientDialect::Fluxer {
        let missing_ids: Vec<String> = snapshots
            .iter()
            .filter(|s| s.partial_user.is_none())
            .map(|s| s.user_id.clone())
            .collect();
        fetch_partial_users_batched(state, &missing_ids).await
    } else {
        HashMap::new()
    };

    snapshots
        .iter()
        .map(|snap| format_presence_json(snap, &fetched, guild_id, dialect))
        .collect()
}

pub async fn handle(session_id: &str, state: &AppState, payload: PresenceUpdatePayload) {
    if !VALID_STATUSES.contains(&payload.status.as_str()) && payload.status != "offline" {
        return;
    }

    let Some(session) = state.hub.get(session_id) else {
        return;
    };

    let session_status = if payload.status == "offline" {
        "offline".to_string()
    } else {
        payload.status.clone()
    };

    let Some((user_id, guild_ids)) =
        session.update_presence(session_status, payload.afk, payload.mobile)
    else {
        return;
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

    let custom_status = if let Some(cs_opt) = payload.custom_status {
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
        state
            .hub
            .set_custom_status(&user_id, extras.custom_status.clone());
        extras.custom_status
    } else {
        state.hub.get_custom_status(&user_id)
    };

    dispatch_sessions_replace(state, &user_id).await;

    let pub_status = user_public_status(state, &user_id).await;
    broadcast_presence(
        state,
        &user_id,
        &guild_ids,
        pub_status,
        custom_status.as_ref(),
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

    let cached_user = match state.hub.get_cached_partial_user(user_id) {
        Some(u) => Some(u),
        None => user_service::get_partial_user(state, user_id).await.ok(),
    };

    broadcast_presence_with_user(
        state,
        user_id,
        guild_ids,
        public_status,
        custom_status,
        cached_user.as_ref(),
    )
    .await;
}

pub async fn broadcast_presence_with_user(
    state: &AppState,
    user_id: &str,
    guild_ids: &HashSet<String>,
    public_status: &str,
    custom_status: Option<&CustomStatus>,
    partial_user: Option<&PartialUser>,
) {
    if guild_ids.is_empty() {
        return;
    }

    let peers = state.hub.sessions_for_guilds_except(guild_ids, user_id);
    if peers.is_empty() {
        return;
    }

    let (_, afk, mobile) = collect_user_live_state(state, user_id).await;
    let custom_status = if public_status == "offline" {
        None
    } else {
        custom_status
    };

    let fluxer_user_val = partial_user
        .and_then(|u| serde_json::to_value(u).ok())
        .unwrap_or_else(|| json!({ "id": user_id }));

    for (peer, shared_guilds) in peers {
        let dialect = peer.dialect();
        for guild_id in shared_guilds {
            let payload = match dialect {
                ClientDialect::Fluxer => json!({
                    "guild_id": guild_id,
                    "user": fluxer_user_val,
                    "status": public_status,
                    "mobile": mobile,
                    "afk": afk,
                    "custom_status": custom_status,
                }),
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
            peer.dispatch(events::PRESENCE_UPDATE, payload);
        }
    }
}