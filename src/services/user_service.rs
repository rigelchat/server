use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::gateway::events;
use crate::error::AppError;
use crate::models::rest::users::{
    UpdateProfileRequest, UpdateSettingsRequest, UpdateUserRequest, UserProfileResponse,
};
use crate::models::user::{
    CurrentUser, UserProfileData, UserProfileRow, UserRow, UserSettings, UserSettingsRow,
};

pub async fn get_current_user(state: &AppState, user_id: &str) -> Result<CurrentUser, AppError> {
    let user = sqlx::query_as::<_, UserRow>("SELECT * FROM users WHERE id = ?")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let profile = sqlx::query_as::<_, UserProfileRow>(
        "SELECT bio, pronouns, accent_color, theme_color_primary, theme_color_secondary FROM user_profiles WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(CurrentUser::from_row(user, profile))
}

pub async fn update_user(
    state: &AppState,
    user_id: &str,
    payload: UpdateUserRequest,
) -> Result<CurrentUser, AppError> {
    let mut query = QueryBuilder::<MySql>::new("UPDATE users SET ");
    let mut separated = query.separated(", ");
    let mut has_updates = false;

    if let Some(global_name) = &payload.global_name {
        separated.push("global_name = ");
        if let Some(name) = global_name {
            let trimmed = name.trim();
            if trimmed.is_empty() || trimmed.len() > 32 {
                return Err(AppError::BadRequest(
                    "global_name must be between 1 and 32 characters".to_string(),
                ));
            }
            separated.push_bind_unseparated(Some(trimmed.to_string()));
        } else {
            separated.push_bind_unseparated(Option::<String>::None);
        }
        has_updates = true;
    }

    if let Some(avatar) = &payload.avatar {
        separated.push("avatar = ");
        separated.push_bind_unseparated(avatar.clone());
        has_updates = true;
    }

    if let Some(banner) = &payload.banner {
        separated.push("banner = ");
        separated.push_bind_unseparated(banner.clone());
        has_updates = true;
    }

    if !has_updates {
        return Err(AppError::BadRequest("No fields to update".to_string()));
    }

    query.push(" WHERE id = ");
    query.push_bind(user_id);
    query.build().execute(&state.db).await?;

    let updated = get_current_user(state, user_id).await?;
    state.dispatch_user(events::USER_UPDATE, &updated, user_id).await;

    Ok(updated)
}

pub async fn get_profile(state: &AppState, user_id: &str) -> Result<UserProfileResponse, AppError> {
    let profile = sqlx::query_as::<_, UserProfileRow>(
        "SELECT bio, pronouns, accent_color, theme_color_primary, theme_color_secondary FROM user_profiles WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("User profile not found".to_string()))?;

    Ok(UserProfileResponse {
        user_profile: UserProfileData::from(profile),
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
        if bio.len() > 190 {
            return Err(AppError::BadRequest("bio must be at most 190 characters".to_string()));
        }
        separated.push("bio = ");
        separated.push_bind_unseparated(bio);
        has_updates = true;
    }

    if let Some(pronouns) = &payload.pronouns {
        if pronouns.len() > 40 {
            return Err(AppError::BadRequest(
                "pronouns must be at most 40 characters".to_string(),
            ));
        }
        separated.push("pronouns = ");
        separated.push_bind_unseparated(pronouns);
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
        query.push(" WHERE id = ");
        query.push_bind(user_id);
        query.build().execute(&state.db).await?;
    }

    get_profile(state, user_id).await
}

pub async fn get_settings(state: &AppState, user_id: &str) -> Result<UserSettings, AppError> {
    let row = sqlx::query_as::<_, UserSettingsRow>(
        "SELECT status, afk_timeout, locale, theme, background_gradient_preset, developer_mode, synced_preferences FROM user_settings WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(row.map(UserSettings::from).unwrap_or_default())
}

pub async fn update_settings(
    state: &AppState,
    user_id: &str,
    payload: UpdateSettingsRequest,
) -> Result<UserSettings, AppError> {
    let mut query = QueryBuilder::<MySql>::new("UPDATE user_settings SET ");
    let mut separated = query.separated(", ");
    let mut has_updates = false;

    if let Some(status) = &payload.status {
        separated.push("status = ");
        separated.push_bind_unseparated(status);
        has_updates = true;
    }

    if let Some(locale) = &payload.locale {
        separated.push("locale = ");
        separated.push_bind_unseparated(locale);
        has_updates = true;
    }

    if let Some(theme) = &payload.theme {
        separated.push("theme = ");
        separated.push_bind_unseparated(theme);
        has_updates = true;
    }

    if let Some(preset) = &payload.background_gradient_preset {
        separated.push("background_gradient_preset = ");
        separated.push_bind_unseparated(preset.clone());
        has_updates = true;
    }

    if let Some(dev_mode) = payload.developer_mode {
        separated.push("developer_mode = ");
        separated.push_bind_unseparated(dev_mode);
        has_updates = true;
    }

    if let Some(synced) = &payload.synced_preferences {
        separated.push("synced_preferences = ");
        separated.push_bind_unseparated(synced);
        has_updates = true;
    }

    if has_updates {
        query.push(" WHERE id = ");
        query.push_bind(user_id);
        query.build().execute(&state.db).await?;
    }

    let settings = get_settings(state, user_id).await?;
    state
        .dispatch_user(events::USER_SETTINGS_UPDATE, &settings, user_id)
        .await;

    Ok(settings)
}