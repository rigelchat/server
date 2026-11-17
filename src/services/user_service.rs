use axum::http::StatusCode;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::gateway::events;
use crate::error::{ApiErrorCode, AppError};
use crate::models::rest::users::{
    GetProfileQuery, MutualGuild, UpdateProfileRequest, UpdateSettingsRequest, UpdateUserRequest,
    UserProfileResponse,
};
use crate::models::user::{
    CurrentUser, CustomStatus, PartialUser, UserProfileData, UserProfileRow, UserRow,
    UserSettings, UserSettingsExtras, UserSettingsRow,
};
use crate::services::auth_service;

#[derive(FromRow)]
struct MutualGuildRow {
    id: String,
    nick: Option<String>,
}

pub async fn get_current_user(state: &AppState, user_id: &str) -> Result<CurrentUser, AppError> {
    let user = sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::coded(StatusCode::NOT_FOUND, ApiErrorCode::UnknownUser, "Unknown user")
        })?;

    let profile = sqlx::query_as::<_, UserProfileRow>(
        "SELECT bio, pronouns, accent_color, theme_color_primary, theme_color_secondary FROM user_profiles WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(CurrentUser::from_row(user, profile))
}

pub async fn get_partial_user(state: &AppState, user_id: &str) -> Result<PartialUser, AppError> {
    let user = sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::coded(StatusCode::NOT_FOUND, ApiErrorCode::UnknownUser, "Unknown user")
        })?;

    Ok(PartialUser::from(&user))
}

pub async fn update_user(
    state: &AppState,
    user_id: &str,
    payload: UpdateUserRequest,
) -> Result<CurrentUser, AppError> {
    let mut tx = state.db.begin().await?;

    let mut user_query = QueryBuilder::<MySql>::new("UPDATE users SET ");
    let mut user_sep = user_query.separated(", ");
    let mut has_user_updates = false;

    if let Some(raw_username) = &payload.username {
        let username = raw_username.trim();
        if !auth_service::is_username_available(state, username).await? {
            let own: Option<(String,)> = sqlx::query_as(
                "SELECT id FROM users WHERE LOWER(username) = LOWER(?) AND id = ? LIMIT 1",
            )
            .bind(username)
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;

            if own.is_none() {
                return Err(AppError::validation(
                    "username",
                    "USERNAME_ALREADY_TAKEN",
                    "This username is already taken.",
                ));
            }
        }
        user_sep.push("username = ");
        user_sep.push_bind_unseparated(username);
        has_user_updates = true;
    }

    if let Some(global_name) = &payload.global_name {
        user_sep.push("global_name = ");
        if let Some(name) = global_name {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                user_sep.push_bind_unseparated(Option::<String>::None);
            } else if trimmed.len() > 32 {
                return Err(AppError::validation(
                    "global_name",
                    "GLOBAL_NAME_LENGTH_INVALID",
                    "Global name must be between 1 and 32 characters.",
                ));
            } else {
                user_sep.push_bind_unseparated(Some(trimmed.to_string()));
            }
        } else {
            user_sep.push_bind_unseparated(Option::<String>::None);
        }
        has_user_updates = true;
    }

    if let Some(avatar) = &payload.avatar {
        user_sep.push("avatar = ");
        user_sep.push_bind_unseparated(avatar.clone());
        has_user_updates = true;
    }

    if let Some(banner) = &payload.banner {
        user_sep.push("banner = ");
        user_sep.push_bind_unseparated(banner.clone());
        has_user_updates = true;
    }

    if has_user_updates {
        user_query.push(" WHERE id = ");
        user_query.push_bind(user_id);
        user_query.build().execute(&mut *tx).await?;
    }

    let mut profile_query = QueryBuilder::<MySql>::new("UPDATE user_profiles SET ");
    let mut profile_sep = profile_query.separated(", ");
    let mut has_profile_updates = false;

    if let Some(bio) = &payload.bio {
        let value = bio.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(text) = value {
            if text.len() > 320 {
                return Err(AppError::validation(
                    "bio",
                    "CONTENT_EXCEEDS_MAX_LENGTH",
                    "Text must not exceed 320 characters.",
                ));
            }
        }
        profile_sep.push("bio = ");
        profile_sep.push_bind_unseparated(value.map(str::to_string));
        has_profile_updates = true;
    }

    if let Some(pronouns) = &payload.pronouns {
        let value = pronouns.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(text) = value {
            if text.len() > 40 {
                return Err(AppError::validation(
                    "pronouns",
                    "CONTENT_EXCEEDS_MAX_LENGTH",
                    "Text must not exceed 40 characters.",
                ));
            }
        }
        profile_sep.push("pronouns = ");
        profile_sep.push_bind_unseparated(value.map(str::to_string));
        has_profile_updates = true;
    }

    if let Some(accent_color) = payload.accent_color {
        if let Some(color) = accent_color {
            if color > 0x00FF_FFFF {
                return Err(AppError::validation(
                    "accent_color",
                    "COLOR_VALUE_TOO_HIGH",
                    "Color value must not exceed 0xffffff.",
                ));
            }
        }
        profile_sep.push("accent_color = ");
        profile_sep.push_bind_unseparated(accent_color);
        has_profile_updates = true;
    }

    if has_profile_updates {
        sqlx::query("INSERT IGNORE INTO user_profiles (id) VALUES (?)")
            .bind(user_id)
            .execute(&mut *tx)
            .await?;

        profile_query.push(" WHERE id = ");
        profile_query.push_bind(user_id);
        profile_query.build().execute(&mut *tx).await?;
    }

    tx.commit().await?;

    let updated = get_current_user(state, user_id).await?;
    if has_user_updates || has_profile_updates {
        state.dispatch_user(events::USER_UPDATE, &updated, user_id).await;
    }

    Ok(updated)
}

pub async fn get_profile(
    state: &AppState,
    viewer_id: &str,
    target_id: &str,
    query: GetProfileQuery,
) -> Result<UserProfileResponse, AppError> {
    let user = get_partial_user(state, target_id).await?;

    let profile = sqlx::query_as::<_, UserProfileRow>(
        "SELECT bio, pronouns, accent_color, theme_color_primary, theme_color_secondary FROM user_profiles WHERE id = ?",
    )
    .bind(target_id)
    .fetch_optional(&state.db)
    .await?
    .map(UserProfileData::from)
    .unwrap_or(UserProfileData {
        bio: None,
        pronouns: None,
        banner: None,
        banner_color: None,
        accent_color: None,
        theme_colors: None,
    });

    let mutual_friends = if query.with_mutual_friends.unwrap_or(false) && viewer_id != target_id {
        Some(vec![])
    } else {
        None
    };

    let mutual_guilds = if query.with_mutual_guilds.unwrap_or(false) && viewer_id != target_id {
        let rows = sqlx::query_as::<_, MutualGuildRow>(
            r#"
            SELECT gm_target.guild_id AS id, gm_target.nick
            FROM guild_members gm_target
            INNER JOIN guild_members gm_viewer
                ON gm_viewer.guild_id = gm_target.guild_id AND gm_viewer.user_id = ?
            WHERE gm_target.user_id = ?
            "#,
        )
        .bind(viewer_id)
        .bind(target_id)
        .fetch_all(&state.db)
        .await?;

        Some(
            rows.into_iter()
                .map(|r| MutualGuild {
                    id: r.id,
                    nick: r.nick,
                })
                .collect(),
        )
    } else {
        None
    };

    Ok(UserProfileResponse {
        user,
        user_profile: profile,
        profile_badges: vec![],
        mutual_friends,
        mutual_guilds,
        connected_accounts: vec![],
        timezone_offset: None,
    })
}

pub async fn update_profile(
    state: &AppState,
    user_id: &str,
    payload: UpdateProfileRequest,
) -> Result<UserProfileResponse, AppError> {
    let mut query = QueryBuilder::<MySql>::new("UPDATE user_profiles SET ");
    let mut separated = query.separated(", ");
    let mut has_updates = false;

    if let Some(bio) = &payload.bio {
        let value = bio.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(text) = value {
            if text.len() > 320 {
                return Err(AppError::validation(
                    "bio",
                    "CONTENT_EXCEEDS_MAX_LENGTH",
                    "Text must not exceed 320 characters.",
                ));
            }
        }
        separated.push("bio = ");
        separated.push_bind_unseparated(value.map(str::to_string));
        has_updates = true;
    }

    if let Some(pronouns) = &payload.pronouns {
        let value = pronouns.as_deref().map(str::trim).filter(|s| !s.is_empty());
        if let Some(text) = value {
            if text.len() > 40 {
                return Err(AppError::validation(
                    "pronouns",
                    "CONTENT_EXCEEDS_MAX_LENGTH",
                    "Text must not exceed 40 characters.",
                ));
            }
        }
        separated.push("pronouns = ");
        separated.push_bind_unseparated(value.map(str::to_string));
        has_updates = true;
    }

    if let Some(accent_color) = payload.accent_color {
        separated.push("accent_color = ");
        separated.push_bind_unseparated(accent_color);
        has_updates = true;
    }

    if let Some(theme_colors) = &payload.theme_colors {
        let primary = theme_colors.first().copied().flatten();
        let secondary = theme_colors.get(1).copied().flatten();
        separated.push("theme_color_primary = ");
        separated.push_bind_unseparated(primary);
        separated.push("theme_color_secondary = ");
        separated.push_bind_unseparated(secondary);
        has_updates = true;
    }

    if has_updates {
        sqlx::query("INSERT IGNORE INTO user_profiles (id) VALUES (?)")
            .bind(user_id)
            .execute(&state.db)
            .await?;

        query.push(" WHERE id = ");
        query.push_bind(user_id);
        query.build().execute(&state.db).await?;

        if let Ok(current) = get_current_user(state, user_id).await {
            state.dispatch_user(events::USER_UPDATE, &current, user_id).await;
        }
    }

    get_profile(
        state,
        user_id,
        user_id,
        GetProfileQuery {
            with_mutual_friends: None,
            with_mutual_guilds: None,
        },
    )
    .await
}

pub async fn get_settings(state: &AppState, user_id: &str) -> Result<UserSettings, AppError> {
    let row = sqlx::query_as::<_, UserSettingsRow>(
        "SELECT status, afk_timeout, locale, theme, background_gradient_preset, developer_mode, synced_preferences, extra_settings FROM user_settings WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    let guild_ids: Vec<String> =
        sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();

    Ok(row
        .map(|r| UserSettings::from_row(r, guild_ids))
        .unwrap_or_default())
}

pub async fn update_settings(
    state: &AppState,
    user_id: &str,
    payload: UpdateSettingsRequest,
) -> Result<UserSettings, AppError> {
    sqlx::query("INSERT IGNORE INTO user_settings (id) VALUES (?)")
        .bind(user_id)
        .execute(&state.db)
        .await?;

    let existing_row = sqlx::query_as::<_, UserSettingsRow>(
        "SELECT status, afk_timeout, locale, theme, background_gradient_preset, developer_mode, synced_preferences, extra_settings FROM user_settings WHERE id = ?",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;

    let mut extras: UserSettingsExtras = existing_row
        .extra_settings
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let mut query = QueryBuilder::<MySql>::new("UPDATE user_settings SET ");
    let mut separated = query.separated(", ");
    let mut has_col_updates = false;
    let mut status_changed = false;
    let mut locale_changed = false;

    if let Some(status) = &payload.status {
        if !matches!(status.as_str(), "online" | "idle" | "dnd" | "invisible") {
            return Err(AppError::validation(
                "status",
                "INVALID_FORMAT",
                "Invalid status value.",
            ));
        }
        status_changed = existing_row.status != *status;
        separated.push("status = ");
        separated.push_bind_unseparated(status);
        has_col_updates = true;
    }

    if let Some(afk_timeout) = payload.afk_timeout {
        separated.push("afk_timeout = ");
        separated.push_bind_unseparated(afk_timeout.clamp(60, 600));
        has_col_updates = true;
    }

    if let Some(locale) = &payload.locale {
        locale_changed = existing_row.locale != *locale;
        separated.push("locale = ");
        separated.push_bind_unseparated(locale);
        has_col_updates = true;
    }

    if let Some(theme) = &payload.theme {
        separated.push("theme = ");
        separated.push_bind_unseparated(theme);
        has_col_updates = true;
    }

    if let Some(preset) = &payload.background_gradient_preset {
        separated.push("background_gradient_preset = ");
        separated.push_bind_unseparated(preset.clone());
        has_col_updates = true;
    }

    if let Some(dev_mode) = payload.developer_mode {
        separated.push("developer_mode = ");
        separated.push_bind_unseparated(dev_mode);
        has_col_updates = true;
    }

    if let Some(synced) = &payload.synced_preferences {
        separated.push("synced_preferences = ");
        separated.push_bind_unseparated(synced.clone().unwrap_or_default());
        has_col_updates = true;
    }

    let mut has_extra_updates = false;
    macro_rules! set_extra {
        ($field:ident, $val:expr) => {
            if let Some(v) = $val {
                extras.$field = v;
                has_extra_updates = true;
            }
        };
    }

    if let Some(cs) = payload.custom_status {
        extras.custom_status = cs.map(CustomStatus::from);
        has_extra_updates = true;
    }
    if let Some(rf) = payload.regional_format {
        extras.regional_format = rf;
        has_extra_updates = true;
    }
    if let Some(rt) = payload.status_resets_to {
        extras.status_resets_to = rt;
        has_extra_updates = true;
    }
    if let Some(ra) = payload.status_resets_at {
        extras.status_resets_at = ra.and_then(|v| match v {
            serde_json::Value::String(s) => Some(s),
            serde_json::Value::Number(n) => n
                .as_i64()
                .and_then(chrono::DateTime::from_timestamp_millis)
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
            _ => None,
        });
        has_extra_updates = true;
    }

    set_extra!(inline_attachment_media, payload.inline_attachment_media);
    set_extra!(inline_embed_media, payload.inline_embed_media);
    set_extra!(gif_auto_play, payload.gif_auto_play);
    set_extra!(render_embeds, payload.render_embeds);
    set_extra!(render_reactions, payload.render_reactions);
    set_extra!(animate_emoji, payload.animate_emoji);
    set_extra!(animate_stickers, payload.animate_stickers);
    set_extra!(render_spoilers, payload.render_spoilers);
    set_extra!(message_display_compact, payload.message_display_compact);
    set_extra!(time_format, payload.time_format);
    set_extra!(timestamp_format, payload.timestamp_format);
    set_extra!(incoming_call_flags, payload.incoming_call_flags);
    set_extra!(group_dm_add_permission_flags, payload.group_dm_add_permission_flags);
    set_extra!(profile_privacy, payload.profile_privacy);
    set_extra!(default_hide_muted_channels, payload.default_hide_muted_channels);
    set_extra!(allow_mature_content_on_ios, payload.allow_mature_content_on_ios);
    set_extra!(default_guilds_restricted, payload.default_guilds_restricted);
    set_extra!(bot_default_guilds_restricted, payload.bot_default_guilds_restricted);
    set_extra!(restricted_guilds, payload.restricted_guilds);
    set_extra!(bot_restricted_guilds, payload.bot_restricted_guilds);
    set_extra!(trusted_domains, payload.trusted_domains);

    if let Some(fsf) = payload.friend_source_flags.or(payload.flags) {
        extras.friend_source_flags = fsf;
        has_extra_updates = true;
    }
    if let Some(folders) = payload.guild_folders {
        extras.guild_folders = Some(folders);
        has_extra_updates = true;
    }

    if has_extra_updates {
        if let Ok(json_str) = serde_json::to_string(&extras) {
            separated.push("extra_settings = ");
            separated.push_bind_unseparated(json_str);
            has_col_updates = true;
        }
    }

    if has_col_updates {
        query.push(" WHERE id = ");
        query.push_bind(user_id);
        query.build().execute(&state.db).await?;
    }

    let settings = get_settings(state, user_id).await?;
    state
        .dispatch_user(events::USER_SETTINGS_UPDATE, &settings, user_id)
        .await;

    if locale_changed {
        if let Ok(current) = get_current_user(state, user_id).await {
            state.dispatch_user(events::USER_UPDATE, &current, user_id).await;
        }
    }

    if status_changed {
        broadcast_presence_from_settings(state, user_id, &settings).await;
    }

    Ok(settings)
}

async fn broadcast_presence_from_settings(
    state: &AppState,
    user_id: &str,
    settings: &UserSettings,
) {
    let guild_ids: std::collections::HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT guild_id FROM guild_members WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

    let public_status = if settings.status == "invisible" {
        "offline"
    } else {
        settings.status.as_str()
    };

    crate::gateway::handlers::presence::broadcast_presence(
        state,
        user_id,
        &guild_ids,
        public_status,
        settings.custom_status.as_ref(),
    )
    .await;
}
