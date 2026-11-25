use std::collections::{HashMap, HashSet};

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tracing::{debug, error, warn};

use crate::AppState;
use crate::constants::flags::SessionFlags;
use crate::constants::gateway::{
    MAX_GATEWAY_SESSIONS_PER_USER, READY_VERSION, close_codes, events,
};
use crate::constants::limits::MAX_SESSION_PROPERTY_LENGTH;
use crate::constants::user::PresenceStatus;
use crate::models::gateway::IdentifyPayload;
use crate::models::gateway::ready::{ReadyData, RtcRegion};
use crate::models::guild::GatewayGuild;
use crate::models::user::{CurrentUser, PartialUser, UserProfileRow, UserRow};
use crate::services::{auth_service, guild, user};
use crate::utils::token::{TokenClaims, strip_auth_prefix, verify};

use super::{lazy_request, presence};

fn hash_session_id(id: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(id.as_bytes()))
}

pub async fn handle(session_id: &str, state: &AppState, payload: IdentifyPayload) {
    let Some(session) = state.hub.get(session_id) else {
        return;
    };
    if session.is_authenticated() {
        session.close(close_codes::ALREADY_AUTHENTICATED);
        return;
    }

    let raw_token = strip_auth_prefix(&payload.token);

    let TokenClaims {
        user_id,
        session_id: auth_session_id,
    } = match verify(raw_token, &state.config.auth.secret) {
        Ok(claims) => claims,
        Err(err) => {
            error!(error = %err, "Identify token validation failed");
            session.close(close_codes::AUTHENTICATION_FAILED);
            return;
        }
    };

    match auth_service::is_session_active(state, &auth_session_id, &user_id).await {
        Ok(Some(_)) => {}
        Ok(None) => {
            session.close(close_codes::AUTHENTICATION_FAILED);
            return;
        }
        Err(err) => {
            error!(error = %err, "Database error verifying session during identify");
            session.close(close_codes::UNKNOWN_ERROR);
            return;
        }
    }

    let user_row = match sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {} FROM users WHERE id = ?",
        UserRow::SELECT_COLS
    ))
    .bind(&user_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(Some(u)) => u,
        Ok(None) => {
            error!(user_id = %user_id, "Identify failed: user does not exist");
            session.close(close_codes::AUTHENTICATION_FAILED);
            return;
        }
        Err(err) => {
            error!(error = %err, "Database error during identify");
            session.close(close_codes::UNKNOWN_ERROR);
            return;
        }
    };

    if !user_row.bot && state.hub.sessions_for_user(&user_id).len() >= MAX_GATEWAY_SESSIONS_PER_USER
    {
        warn!(user_id = %user_id, "Max concurrent gateway sessions reached for user");
        session.close(close_codes::RATE_LIMITED);
        return;
    }

    let os: String = payload
        .properties
        .get("os")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .chars()
        .take(MAX_SESSION_PROPERTY_LENGTH)
        .collect();
    let browser: String = payload
        .properties
        .get("browser")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .chars()
        .take(MAX_SESSION_PROPERTY_LENGTH)
        .collect();

    if let Err(err) =
        sqlx::query("UPDATE user_sessions SET last_used_at = ?, os = ?, platform = ? WHERE id = ?")
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
                error!(error = %err, "Failed to load user guilds during identify");
                session.close(close_codes::UNKNOWN_ERROR);
                return;
            }
        };

    let profile = match sqlx::query_as::<_, UserProfileRow>(
        "SELECT bio, pronouns, accent_color, theme_color_primary, theme_color_secondary FROM user_profiles WHERE id = ?",
    )
    .bind(&user_id)
    .fetch_optional(&state.db)
    .await
    {
        Ok(p) => p,
        Err(err) => {
            error!(error = %err, "Failed to load user profile during identify");
            session.close(close_codes::UNKNOWN_ERROR);
            return;
        }
    };

    let user_settings = match user::get_settings(state, &user_id).await {
        Ok(s) => s,
        Err(err) => {
            error!(error = %err, "Failed to load user settings during identify");
            session.close(close_codes::UNKNOWN_ERROR);
            return;
        }
    };

    let saved_status = user_settings.status.clone();
    let prop_mobile = payload
        .properties
        .get("mobile")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let saved_parsed = PresenceStatus::parse(&saved_status).unwrap_or(PresenceStatus::Online);
    let (initial_status, initial_afk, initial_mobile) = if let Some(pres) = &payload.presence {
        if !user_row.bot && pres.status.as_deref().is_some_and(|s| s != saved_status) {
            debug!(
                user_id = %user_id,
                identify_status = ?pres.status,
                saved_status = %saved_status,
                "Ignoring identify presence status in favor of persisted user_settings.status"
            );
        }
        let chosen_status = if user_row.bot {
            match pres.status.as_deref().and_then(PresenceStatus::parse) {
                Some(PresenceStatus::Offline) | None => PresenceStatus::Online,
                Some(st) => st,
            }
        } else {
            saved_parsed
        };
        (
            chosen_status,
            pres.afk.unwrap_or(false),
            pres.mobile.unwrap_or(prop_mobile),
        )
    } else {
        (saved_parsed, false, prop_mobile)
    };

    let session_flags = payload.flags.unwrap_or(0);
    let is_bot = user_row.bot;
    let can_use_threads =
        is_bot || SessionFlags::from_bits_truncate(session_flags).contains(SessionFlags::CHANNEL_THREADS);
    let custom_status = user_settings.custom_status.clone();
    let user_summary = PartialUser::from(&user_row);
    let current_user = CurrentUser::from_row(user_row, profile);

    let Some(session) = state.hub.authenticate(
        session_id,
        user_id.clone(),
        auth_session_id.clone(),
        is_bot,
        session_flags,
        initial_status,
        initial_afk,
        initial_mobile,
        &guild_ids,
        user_summary.clone(),
        custom_status.clone(),
    ) else {
        return;
    };

    let guilds: Vec<GatewayGuild> =
        match guild::build_gateway_guilds_batch(state, &guild_ids, &user_id, can_use_threads, is_bot)
            .await
        {
            Ok(g) => g,
            Err(err) => {
                error!(error = %err, "Failed to build gateway guilds batch");
                session.close(close_codes::UNKNOWN_ERROR);
                return;
            }
        };

    debug!(user_id = %user_id, "User identified, sending READY");

    let ready = ReadyData {
        auth_session_id_hash: hash_session_id(&auth_session_id),
        session_id: session_id.to_string(),
        country_code: state.config.instance.country_code.clone(),
        version: READY_VERSION,
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

    session.dispatch(events::READY, ready);

    let pub_status = presence::user_public_status(state, &user_id).await;
    let my_guilds: HashSet<String> = guild_ids.into_iter().collect();
    presence::dispatch_sessions_replace(state, &user_id).await;
    let is_sole_offline_session =
        pub_status == "offline" && state.hub.sessions_for_user(&user_id).len() <= 1;
    if !is_sole_offline_session {
        presence::broadcast_presence(
            state,
            &user_id,
            &my_guilds,
            pub_status,
            custom_status.as_ref(),
        )
        .await;
        let active_guilds: Vec<&String> = my_guilds
            .iter()
            .filter(|gid| !state.hub.guild_member_list_subscribers(gid).is_empty())
            .collect();
        futures_util::future::join_all(
            active_guilds
                .into_iter()
                .map(|gid| lazy_request::refresh_guild_member_lists(state, gid)),
        )
        .await;
    }
}
