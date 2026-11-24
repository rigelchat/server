use std::collections::HashSet;

use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::gateway::events;
use crate::constants::limits::{
    MAX_ASSET_HASH_LENGTH, MAX_EXTRA_SETTINGS_BYTES, MAX_FOLDER_ICON_LENGTH,
    MAX_FOLDER_NAME_LENGTH, MAX_GRADIENT_PRESET_LENGTH, MAX_GUILD_FOLDERS, MAX_LOCALE_LENGTH,
    MAX_SETTINGS_LIST_ITEMS, MAX_SYNCED_PREFS_BYTES, MIN_AFK_TIMEOUT,
};
use crate::constants::user::{DEFAULT_AFK_TIMEOUT, VALID_STATUSES, VALID_THEMES};
use crate::error::{AppError, ValidationCode};
use crate::models::rest::users::UpdateSettingsRequest;
use crate::models::user::{
    CustomStatusInput, UserSettings, UserSettingsExtras, UserSettingsRow, reconcile_guild_folders,
};
use crate::utils::snowflake::validate_snowflake;

use super::profile::get_current_user;

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

    Ok(match row {
        Some(r) => UserSettings::from_row(r, guild_ids),
        None => {
            let mut s = UserSettings::default();
            reconcile_guild_folders(&mut s.guild_folders, &guild_ids);
            s
        }
    })
}

pub async fn update_settings(
    state: &AppState,
    user_id: &str,
    payload: UpdateSettingsRequest,
) -> Result<UserSettings, AppError> {
    let mut tx = state.db.begin().await?;

    sqlx::query("INSERT IGNORE INTO user_settings (id) VALUES (?)")
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    let existing_row = sqlx::query_as::<_, UserSettingsRow>(
        "SELECT status, afk_timeout, locale, theme, background_gradient_preset, developer_mode, synced_preferences, extra_settings FROM user_settings WHERE id = ? FOR UPDATE",
    )
    .bind(user_id)
    .fetch_one(&mut *tx)
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
        if !VALID_STATUSES.contains(&status.as_str()) {
            return Err(AppError::validation(
                "status",
                ValidationCode::InvalidFormat,
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
        separated.push_bind_unseparated(afk_timeout.clamp(MIN_AFK_TIMEOUT, DEFAULT_AFK_TIMEOUT));
        has_col_updates = true;
    }

    if let Some(locale) = &payload.locale {
        let trimmed = locale.trim();
        if trimmed.is_empty()
            || trimmed.len() > MAX_LOCALE_LENGTH
            || !trimmed
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(AppError::validation(
                "locale",
                ValidationCode::InvalidFormat,
                "Invalid locale format.",
            ));
        }
        locale_changed = existing_row.locale != trimmed;
        separated.push("locale = ");
        separated.push_bind_unseparated(trimmed.to_string());
        has_col_updates = true;
    }

    if let Some(theme) = &payload.theme {
        if !VALID_THEMES.contains(&theme.as_str()) {
            return Err(AppError::validation(
                "theme",
                ValidationCode::InvalidFormat,
                "Invalid theme value.",
            ));
        }
        separated.push("theme = ");
        separated.push_bind_unseparated(theme);
        has_col_updates = true;
    }

    if let Some(preset) = &payload.background_gradient_preset {
        let value = preset
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(ref v) = value
            && (v.len() > MAX_GRADIENT_PRESET_LENGTH || v.chars().any(char::is_control))
        {
            return Err(AppError::validation(
                "background_gradient_preset",
                ValidationCode::StringLengthInvalid,
                "background_gradient_preset must not exceed 32 characters.",
            ));
        }
        separated.push("background_gradient_preset = ");
        separated.push_bind_unseparated(value);
        has_col_updates = true;
    }

    if let Some(dev_mode) = payload.developer_mode {
        separated.push("developer_mode = ");
        separated.push_bind_unseparated(dev_mode);
        has_col_updates = true;
    }

    if let Some(synced) = &payload.synced_preferences {
        let value = synced.clone().unwrap_or_default();
        if value.len() > MAX_SYNCED_PREFS_BYTES {
            return Err(AppError::validation(
                "synced_preferences",
                ValidationCode::ContentExceedsMaxLength,
                "synced_preferences exceeds maximum size.",
            ));
        }
        separated.push("synced_preferences = ");
        separated.push_bind_unseparated(value);
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

    let custom_status_changed = payload.custom_status.is_some();
    if let Some(cs) = payload.custom_status {
        extras.custom_status = cs
            .map(CustomStatusInput::try_into_custom_status)
            .transpose()?;
        has_extra_updates = true;
    }
    if let Some(rf) = payload.regional_format {
        let value = rf
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(ref v) = value
            && (v.len() > MAX_LOCALE_LENGTH
                || !v
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
        {
            return Err(AppError::validation(
                "regional_format",
                ValidationCode::InvalidFormat,
                "Invalid regional_format value.",
            ));
        }
        extras.regional_format = value;
        has_extra_updates = true;
    }
    if let Some(rt) = payload.status_resets_to {
        if let Some(ref s) = rt
            && !VALID_STATUSES.contains(&s.as_str())
        {
            return Err(AppError::validation(
                "status_resets_to",
                ValidationCode::InvalidFormat,
                "Invalid status_resets_to value.",
            ));
        }
        extras.status_resets_to = rt;
        has_extra_updates = true;
    }
    if let Some(ra) = payload.status_resets_at {
        extras.status_resets_at = ra
            .map(|v| v.into_opt_iso("status_resets_at"))
            .transpose()?
            .flatten();
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
    set_extra!(
        group_dm_add_permission_flags,
        payload.group_dm_add_permission_flags
    );
    set_extra!(profile_privacy, payload.profile_privacy);
    set_extra!(
        default_hide_muted_channels,
        payload.default_hide_muted_channels
    );
    set_extra!(
        allow_mature_content_on_ios,
        payload.allow_mature_content_on_ios
    );
    set_extra!(default_guilds_restricted, payload.default_guilds_restricted);
    set_extra!(
        bot_default_guilds_restricted,
        payload.bot_default_guilds_restricted
    );

    if let Some(guilds) = payload.restricted_guilds {
        validate_guild_id_list("restricted_guilds", &guilds)?;
        extras.restricted_guilds = guilds;
        has_extra_updates = true;
    }
    if let Some(guilds) = payload.bot_restricted_guilds {
        validate_guild_id_list("bot_restricted_guilds", &guilds)?;
        extras.bot_restricted_guilds = guilds;
        has_extra_updates = true;
    }
    if let Some(domains) = payload.trusted_domains {
        if domains.len() > MAX_SETTINGS_LIST_ITEMS
            || domains.iter().any(|d| {
                d.trim().is_empty()
                    || d.len() > MAX_ASSET_HASH_LENGTH
                    || d.chars().any(char::is_control)
            })
        {
            return Err(AppError::validation(
                "trusted_domains",
                ValidationCode::InvalidFormat,
                "Invalid trusted_domains list.",
            ));
        }
        extras.trusted_domains = domains;
        has_extra_updates = true;
    }

    if let Some(fsf) = payload.friend_source_flags.or(payload.flags) {
        extras.friend_source_flags = fsf;
        has_extra_updates = true;
    }
    if let Some(mut folders) = payload.guild_folders {
        if folders.len() > MAX_GUILD_FOLDERS {
            return Err(AppError::validation(
                "guild_folders",
                ValidationCode::InvalidFormat,
                "Too many guild folders.",
            ));
        }
        let mut has_uncat = false;
        let mut next_id = folders
            .iter()
            .map(|f| f.id)
            .filter(|&id| id > 0)
            .max()
            .unwrap_or(0)
            + 1;
        for folder in &mut folders {
            if folder.id < -1 {
                return Err(AppError::validation(
                    "guild_folders",
                    ValidationCode::InvalidFormat,
                    "Invalid guild folder id.",
                ));
            }
            if folder.id == -1 {
                if has_uncat {
                    return Err(AppError::validation(
                        "guild_folders",
                        ValidationCode::InvalidFormat,
                        "Duplicate uncategorized guild folder.",
                    ));
                }
                has_uncat = true;
            } else if folder.id == 0 {
                folder.id = next_id;
                next_id += 1;
            }
            folder.name = folder
                .name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            if folder
                .name
                .as_ref()
                .is_some_and(|n| n.chars().count() > MAX_FOLDER_NAME_LENGTH)
                || folder.icon.len() > MAX_FOLDER_ICON_LENGTH
                || folder.icon.chars().any(char::is_control)
            {
                return Err(AppError::validation(
                    "guild_folders",
                    ValidationCode::InvalidFormat,
                    "Invalid guild folder metadata.",
                ));
            }
            validate_guild_id_list("guild_folders", &folder.guild_ids)?;
        }
        let actual_guild_ids: Vec<String> =
            sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id = ?")
                .bind(user_id)
                .fetch_all(&mut *tx)
                .await?;
        reconcile_guild_folders(&mut folders, &actual_guild_ids);
        extras.guild_folders = Some(folders);
        has_extra_updates = true;
    }

    if has_extra_updates {
        let json_str = serde_json::to_string(&extras)
            .map_err(|err| AppError::internal(format!("Failed to serialize settings: {err}")))?;
        if json_str.len() > MAX_EXTRA_SETTINGS_BYTES {
            return Err(AppError::validation(
                "settings",
                ValidationCode::ContentExceedsMaxLength,
                "Settings payload exceeds maximum allowed size.",
            ));
        }
        separated.push("extra_settings = ");
        separated.push_bind_unseparated(json_str);
        has_col_updates = true;
    }

    if has_col_updates {
        query.push(" WHERE id = ");
        query.push_bind(user_id);
        query.build().execute(&mut *tx).await?;
    }

    tx.commit().await?;

    let settings = get_settings(state, user_id).await?;
    state
        .dispatch_user(events::USER_SETTINGS_UPDATE, &settings, user_id)
        .await;

    if locale_changed && let Ok(current) = get_current_user(state, user_id).await {
        state
            .dispatch_user(events::USER_UPDATE, &current, user_id)
            .await;
    }

    let prev_public_status = if status_changed || custom_status_changed {
        crate::gateway::handlers::presence::user_public_status(state, user_id).await
    } else {
        "offline"
    };

    if let Some(new_status) = &payload.status {
        state.hub.set_user_status(user_id, new_status);
    }
    if custom_status_changed {
        state
            .hub
            .set_custom_status(user_id, settings.custom_status.clone());
    }

    if (status_changed || custom_status_changed) && !state.hub.sessions_for_user(user_id).is_empty()
    {
        crate::gateway::handlers::presence::dispatch_sessions_replace(state, user_id).await;
        let next_public_status =
            crate::gateway::handlers::presence::user_public_status(state, user_id).await;
        if !(prev_public_status == "offline" && next_public_status == "offline") {
            broadcast_presence_from_settings(state, user_id, &settings).await;
        }
    }

    Ok(settings)
}

async fn broadcast_presence_from_settings(
    state: &AppState,
    user_id: &str,
    settings: &UserSettings,
) {
    let guild_ids: HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT guild_id FROM guild_members WHERE user_id = ?")
            .bind(user_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default()
            .into_iter()
            .collect();

    let public_status =
        crate::gateway::handlers::presence::user_public_status(state, user_id).await;

    crate::gateway::handlers::presence::broadcast_presence(
        state,
        user_id,
        &guild_ids,
        public_status,
        settings.custom_status.as_ref(),
    )
    .await;

    for gid in &guild_ids {
        crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, gid).await;
    }
}

fn validate_guild_id_list(field: &str, ids: &[String]) -> Result<(), AppError> {
    if ids.len() > MAX_SETTINGS_LIST_ITEMS {
        return Err(AppError::validation(
            field,
            ValidationCode::InvalidFormat,
            format!("Too many entries in {field}."),
        ));
    }
    for id in ids {
        validate_snowflake(field, id)?;
    }
    Ok(())
}
