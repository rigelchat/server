use std::collections::HashSet;

use serde::Deserialize;
use serde_json::json;
use tracing::error;

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::events;
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

pub async fn handle(session_id: &str, state: &AppState, payload: PresenceUpdatePayload) {
    let stored_status = match payload.status.as_str() {
        "online" | "idle" | "dnd" | "invisible" => payload.status.as_str(),
        "offline" => "invisible",
        _ => return,
    };

    let (user_id, guild_ids) = {
        let sessions = state.sessions.read().await;
        let Some(session) = sessions.get(session_id) else {
            return;
        };
        let Some(uid) = session.user_id.clone() else {
            return;
        };
        (uid, session.guilds.clone())
    };

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

    if let Err(err) = sqlx::query("UPDATE user_settings SET status = ? WHERE id = ?")
        .bind(stored_status)
        .bind(&user_id)
        .execute(&state.db)
        .await
    {
        error!(error = %err, "Failed to update status in user_settings");
    }

    let public_status = if stored_status == "invisible" {
        "offline"
    } else {
        stored_status
    };

    let session_list = vec![ClientSessionInfo {
        session_id: "all".to_string(),
        status: stored_status.to_string(),
        afk: payload.afk.unwrap_or(false),
        mobile: payload.mobile.unwrap_or(false),
    }];
    state
        .dispatch_user(events::SESSIONS_REPLACE, &session_list, &user_id)
        .await;

    broadcast_presence(
        state,
        &user_id,
        &guild_ids,
        public_status,
        extras.custom_status.as_ref(),
    )
    .await;
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
                        "mobile": false,
                        "afk": false,
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