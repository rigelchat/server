use std::collections::{HashMap, HashSet};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use serde_json::json;
use sha2::{Digest, Sha256};
use tracing::{debug, error};

use crate::AppState;
use crate::constants::gateway::{close_codes, events};
use crate::models::gateway::IdentifyPayload;
use crate::models::gateway::ready::{ClientSessionInfo, ReadyData, RtcRegion};
use crate::models::guild::GatewayGuild;
use crate::models::user::{CurrentUser, GatewayUserSummary, UserProfileRow, UserRow};
use crate::services::{guild_service, user_service};
use crate::utils::token::{TokenClaims, verify};

fn hash_session_id(id: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(id.as_bytes()))
}

async fn close_session(state: &AppState, session_id: &str, close: (u16, &'static str)) {
    let sessions = state.sessions.read().await;
    if let Some(session) = sessions.get(session_id) {
        session.close(close).await;
    }
}

pub async fn handle(session_id: &str, state: &AppState, payload: IdentifyPayload) {
    {
        let sessions = state.sessions.read().await;
        if let Some(session) = sessions.get(session_id) {
            if session.user_id.is_some() {
                drop(sessions);
                close_session(state, session_id, close_codes::ALREADY_AUTHENTICATED).await;
                return;
            }
        }
    }

    let TokenClaims {
        user_id,
        session_id: auth_session_id,
    } = match verify(&payload.token, &state.config.auth_secret) {
        Ok(claims) => claims,
        Err(err) => {
            error!(error = %err, "Identify token validation failed");
            close_session(state, session_id, close_codes::AUTHENTICATION_FAILED).await;
            return;
        }
    };

    let user_row = match sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = ?")
        .bind(&user_id)
        .fetch_optional(&state.db)
        .await
    {
        Ok(Some(u)) => u,
        Ok(None) => {
            error!(user_id = %user_id, "Identify failed: user does not exist");
            close_session(state, session_id, close_codes::AUTHENTICATION_FAILED).await;
            return;
        }
        Err(err) => {
            error!(error = %err, "Database error during identify");
            close_session(state, session_id, close_codes::UNKNOWN_ERROR).await;
            return;
        }
    };

    let os = payload
        .properties
        .get("os")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let browser = payload
        .properties
        .get("browser")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if let Err(err) = sqlx::query(
        "UPDATE user_sessions SET last_used_at = ?, os = ?, platform = ? WHERE id = ?",
    )
    .bind(Utc::now().timestamp_millis() as u64)
    .bind(os)
    .bind(browser)
    .bind(&auth_session_id)
    .execute(&state.db)
    .await
    {
        error!(error = %err, "Failed to update user_sessions");
    }

    let guild_ids: Vec<String> =
        match sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id = ?")
            .bind(&user_id)
            .fetch_all(&state.db)
            .await
        {
            Ok(ids) => ids,
            Err(err) => {
                error!(error = %err, "Failed to load user guilds");
                vec![]
            }
        };

    {
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.user_id = Some(user_id.clone());
            session.auth_session_id = Some(auth_session_id.clone());
            session.guilds = guild_ids.iter().cloned().collect();
        }
    }

    let mut guilds: Vec<GatewayGuild> = Vec::with_capacity(guild_ids.len());
    for gid in &guild_ids {
        match guild_service::build_gateway_guild(state, gid, &user_id).await {
            Ok(g) => guilds.push(g),
            Err(err) => error!(error = %err, guild_id = %gid, "Failed to build gateway guild"),
        }
    }

    let profile = sqlx::query_as::<_, UserProfileRow>(
        "SELECT bio, pronouns, accent_color, theme_color_primary, theme_color_secondary FROM user_profiles WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let mut user_settings = user_service::get_settings(state, &user_id)
        .await
        .unwrap_or_default();

    user_settings.guild_folders = vec![json!({
        "id": -1,
        "name": null,
        "color": 0,
        "flags": 0,
        "icon": "folder",
        "guild_ids": guild_ids,
    })];

    let status = user_settings.status.clone();
    let user_summary = GatewayUserSummary::from(&user_row);
    let current_user = CurrentUser::from_row(user_row, profile);

    let ready = ReadyData {
        auth_session_id_hash: hash_session_id(&auth_session_id),
        session_id: session_id.to_string(),
        country_code: "FR".to_string(),
        geoip_country_code: "FR".to_string(),
        version: 1,
        guilds,
        private_channels: vec![],
        pinned_dms: vec![],
        relationships: vec![],
        presences: vec![],
        read_states: vec![],
        notes: HashMap::new(),
        favorite_memes: vec![],
        webauthn_credentials: vec![],
        user_guild_settings: vec![],
        rtc_regions: vec![RtcRegion {
            id: "automatic".to_string(),
            name: "Automatic".to_string(),
            emoji: "🌐".to_string(),
        }],
        sessions: vec![ClientSessionInfo {
            session_id: "all".to_string(),
            status: status.clone(),
            afk: false,
            mobile: false,
        }],
        user: current_user,
        user_settings,
        users: vec![user_summary],
    };

    debug!(user_id = %user_id, "User identified, sending READY");

    let my_guilds: HashSet<String> = guild_ids.into_iter().collect();
    let sessions = state.sessions.read().await;

    if let Some(session) = sessions.get(session_id) {
        session.dispatch(events::READY, ready).await;
    }

    for peer in sessions.values() {
        let Some(peer_uid) = &peer.user_id else {
            continue;
        };
        if peer_uid == &user_id {
            continue;
        }
        for guild_id in peer.guilds.intersection(&my_guilds) {
            peer.dispatch(
                events::PRESENCE_UPDATE,
                json!({
                    "guild_id": guild_id,
                    "user": { "id": user_id },
                    "status": status,
                    "activities": [],
                }),
            )
            .await;
        }
    }
}