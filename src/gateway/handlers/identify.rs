use std::collections::{HashMap, HashSet};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tracing::{debug, error};

use crate::AppState;
use crate::constants::API_VERSION;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::{close_codes, events};
use crate::models::gateway::IdentifyPayload;
use crate::models::gateway::ready::{ DiscordReadyData, DiscordReadyGuild, PartialApplication, ReadyData,
    RtcRegion, UnavailableGuild,
};
use crate::models::guild::{DiscordGuildCreate, GatewayGuild};
use crate::models::user::{CurrentUser, GuildFolder, PartialUser, UserProfileRow, UserRow};
use crate::services::{auth_service, guild_service, user_service};
use crate::utils::token::{TokenClaims, verify};

use super::{lazy_request, presence};

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
    let dialect = {
        let sessions = state.sessions.read().await;
        let Some(session) = sessions.get(session_id) else {
            return;
        };
        if session.user_id.is_some() {
            drop(sessions);
            close_session(state, session_id, close_codes::ALREADY_AUTHENTICATED).await;
            return;
        }
        if payload.intents.is_some() {
            ClientDialect::Discord
        } else {
            session.dialect
        }
    };

    let raw_token = payload
        .token
        .strip_prefix("Bot ")
        .or_else(|| payload.token.strip_prefix("Bearer "))
        .unwrap_or(&payload.token)
        .trim();

    let TokenClaims {
        user_id,
        session_id: auth_session_id,
    } = match verify(raw_token, &state.config.auth.secret) {
        Ok(claims) => claims,
        Err(err) => {
            error!(error = %err, "Identify token validation failed");
            close_session(state, session_id, close_codes::AUTHENTICATION_FAILED).await;
            return;
        }
    };

    if !auth_service::is_session_active(state, &auth_session_id, &user_id)
        .await
        .unwrap_or(false)
    {
        close_session(state, session_id, close_codes::AUTHENTICATION_FAILED).await;
        return;
    }

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
            session.dialect = dialect;
            session.user_id = Some(user_id.clone());
            session.auth_session_id = Some(auth_session_id.clone());
            session.guilds = guild_ids.iter().cloned().collect();
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

    {
        let mut sessions = state.sessions.write().await;
        if let Some(session) = sessions.get_mut(session_id) {
            session.status = user_settings.status.clone();
        }
    }

    user_settings.guild_folders = vec![GuildFolder {
        id: -1,
        name: None,
        color: 0,
        flags: 0,
        icon: "folder".to_string(),
        guild_ids: guild_ids.clone(),
    }];

    let custom_status = user_settings.custom_status.clone();
    let is_bot = user_row.bot;
    let user_summary = PartialUser::from(&user_row);
    let current_user = CurrentUser::from_row(user_row, profile).with_dialect(dialect);

    debug!(user_id = %user_id, ?dialect, "User identified, sending READY");

    match dialect {
        ClientDialect::Fluxer => {
            let mut guilds: Vec<GatewayGuild> = Vec::with_capacity(guild_ids.len());
            for gid in &guild_ids {
                match guild_service::build_gateway_guild(state, gid, &user_id).await {
                    Ok(g) => guilds.push(g),
                    Err(err) => error!(error = %err, guild_id = %gid, "Failed to build gateway guild"),
                }
            }

            let ready = ReadyData {
                auth_session_id_hash: hash_session_id(&auth_session_id),
                session_id: session_id.to_string(),
                country_code: state.config.instance.country_code.clone(),
                version: 1,
                guilds,
                private_channels: vec![],
                pinned_dms: vec![],
                relationships: vec![],
                presences: presence::build_ready_presences(state, &user_id, &guild_ids).await,
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
                sessions: presence::build_user_sessions(state, &user_id).await,
                user: current_user,
                user_settings,
                users: vec![user_summary],
            };

            let sessions = state.sessions.read().await;
            if let Some(session) = sessions.get(session_id) {
                session.dispatch(events::READY, ready).await;
            }
        }
        ClientDialect::Discord => {
            let mut full_guilds: Vec<DiscordGuildCreate> = Vec::with_capacity(guild_ids.len());
            for gid in &guild_ids {
                match guild_service::build_full_gateway_guild(state, gid).await {
                    Ok(g) => full_guilds.push(DiscordGuildCreate::from(g)),
                    Err(err) => error!(error = %err, guild_id = %gid, "Failed to build discord guild"),
                }
            }

            let ready_guilds: Vec<DiscordReadyGuild> = if is_bot {
                guild_ids
                    .iter()
                    .map(|id| {
                        DiscordReadyGuild::Unavailable(UnavailableGuild {
                            id: id.clone(),
                            unavailable: true,
                        })
                    })
                    .collect()
            } else {
                full_guilds
                    .iter()
                    .cloned()
                    .map(|g| DiscordReadyGuild::Full(Box::new(g)))
                    .collect()
            };

            let ready = DiscordReadyData {
                v: API_VERSION,
                user: current_user,
                guilds: ready_guilds,
                session_id: session_id.to_string(),
                resume_gateway_url: format!("{}/gateway", state.config.public_ws_url),
                shard: payload.shard,
                application: is_bot.then(|| PartialApplication {
                    id: user_id.clone(),
                    flags: 0,
                }),
            };

            let sessions = state.sessions.read().await;
            if let Some(session) = sessions.get(session_id) {
                session.dispatch(events::READY, ready).await;
                if is_bot {
                    for g in full_guilds {
                        session.dispatch(events::GUILD_CREATE, g).await;
                    }
                }
            }
        }
    }

    let pub_status = presence::user_public_status(state, &user_id).await;
    let my_guilds: HashSet<String> = guild_ids.into_iter().collect();
    presence::dispatch_sessions_replace(state, &user_id).await;
    presence::broadcast_presence(
        state,
        &user_id,
        &my_guilds,
        pub_status,
        custom_status.as_ref(),
    )
    .await;
    for gid in &my_guilds {
        lazy_request::refresh_guild_member_lists(state, gid).await;
    }
}