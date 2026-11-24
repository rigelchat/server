use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::gateway::events;
use crate::constants::limits::{
    MAX_BIO_LENGTH, MAX_GLOBAL_NAME_LENGTH, MAX_PRONOUNS_LENGTH, MAX_THEME_COLORS,
};
use crate::error::{AppError, ValidationCode};
use crate::models::rest::users::{
    GetProfileQuery, MutualGuild, UpdateProfileRequest, UpdateUserRequest, UserProfileResponse,
};
use crate::models::user::{CurrentUser, PartialUser, UserProfileData, UserProfileRow, UserRow};
use crate::services::auth_service;
use crate::utils::validation::{
    normalize_optional_text, reject_control_chars, validate_asset_opt, validate_color_opt,
};

#[derive(FromRow)]
struct MutualGuildRow {
    id: String,
    nick: Option<String>,
}

pub async fn get_current_user(state: &AppState, user_id: &str) -> Result<CurrentUser, AppError> {
    let user = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {} FROM users WHERE id = ?",
        UserRow::SELECT_COLS
    ))
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_user)?;

    let profile = sqlx::query_as::<_, UserProfileRow>(
        "SELECT bio, pronouns, accent_color, theme_color_primary, theme_color_secondary FROM user_profiles WHERE id = ?",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(CurrentUser::from_row(user, profile))
}

pub async fn get_partial_user(state: &AppState, user_id: &str) -> Result<PartialUser, AppError> {
    let user = sqlx::query_as::<_, UserRow>(&format!(
        "SELECT {} FROM users WHERE id = ?",
        UserRow::SELECT_COLS
    ))
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_user)?;

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
        auth_service::validate_username(username)?;
        let taken_by_other: Option<(String,)> =
            sqlx::query_as("SELECT id FROM users WHERE username = ? AND id != ? LIMIT 1")
                .bind(username)
                .bind(user_id)
                .fetch_optional(&mut *tx)
                .await?;

        if taken_by_other.is_some() {
            return Err(AppError::validation(
                "username",
                ValidationCode::UsernameAlreadyTaken,
                "This username is already taken.",
            ));
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
            } else if trimmed.chars().count() > MAX_GLOBAL_NAME_LENGTH {
                return Err(AppError::validation(
                    "global_name",
                    ValidationCode::GlobalNameLengthInvalid,
                    "Global name must be between 1 and 32 characters.",
                ));
            } else {
                reject_control_chars("global_name", trimmed)?;
                user_sep.push_bind_unseparated(Some(trimmed.to_string()));
            }
        } else {
            user_sep.push_bind_unseparated(Option::<String>::None);
        }
        has_user_updates = true;
    }

    if let Some(avatar) = &payload.avatar {
        let value = validate_asset_opt("avatar", avatar.as_deref())?;
        user_sep.push("avatar = ");
        user_sep.push_bind_unseparated(value);
        has_user_updates = true;
    }

    if let Some(banner) = &payload.banner {
        let value = validate_asset_opt("banner", banner.as_deref())?;
        user_sep.push("banner = ");
        user_sep.push_bind_unseparated(value);
        has_user_updates = true;
    }

    if has_user_updates {
        user_query.push(" WHERE id = ");
        user_query.push_bind(user_id);
        if let Err(err) = user_query.build().execute(&mut *tx).await {
            if let sqlx::Error::Database(db_err) = &err
                && db_err.is_unique_violation()
            {
                return Err(AppError::validation(
                    "username",
                    ValidationCode::UsernameAlreadyTaken,
                    "This username is already taken.",
                ));
            }
            return Err(err.into());
        }
    }

    let mut profile_query = QueryBuilder::<MySql>::new("UPDATE user_profiles SET ");
    let mut profile_sep = profile_query.separated(", ");
    let mut has_profile_updates = false;

    if let Some(bio) = &payload.bio {
        let value = normalize_optional_text("bio", bio.as_deref(), MAX_BIO_LENGTH)?;
        profile_sep.push("bio = ");
        profile_sep.push_bind_unseparated(value);
        has_profile_updates = true;
    }

    if let Some(pronouns) = &payload.pronouns {
        let value = normalize_optional_text("pronouns", pronouns.as_deref(), MAX_PRONOUNS_LENGTH)?;
        profile_sep.push("pronouns = ");
        profile_sep.push_bind_unseparated(value);
        has_profile_updates = true;
    }

    if let Some(accent_color) = payload.accent_color {
        validate_color_opt("accent_color", accent_color)?;
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
    if has_user_updates {
        state.hub.update_partial_user(&PartialUser::from(&updated));
    }
    if has_user_updates || has_profile_updates {
        state
            .dispatch_user(events::USER_UPDATE, &updated, user_id)
            .await;
        if has_user_updates {
            let guild_ids: Vec<String> =
                sqlx::query_scalar("SELECT guild_id FROM guild_members WHERE user_id = ?")
                    .bind(user_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default();
            for gid in guild_ids {
                crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, &gid)
                    .await;
            }
        }
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
        let value = normalize_optional_text("bio", bio.as_deref(), MAX_BIO_LENGTH)?;
        separated.push("bio = ");
        separated.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(pronouns) = &payload.pronouns {
        let value = normalize_optional_text("pronouns", pronouns.as_deref(), MAX_PRONOUNS_LENGTH)?;
        separated.push("pronouns = ");
        separated.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(accent_color) = payload.accent_color {
        validate_color_opt("accent_color", accent_color)?;
        separated.push("accent_color = ");
        separated.push_bind_unseparated(accent_color);
        has_updates = true;
    }

    if let Some(theme_colors) = &payload.theme_colors {
        if theme_colors.len() > MAX_THEME_COLORS {
            return Err(AppError::validation(
                "theme_colors",
                ValidationCode::InvalidFormat,
                "theme_colors must contain at most 2 colors.",
            ));
        }
        let primary = theme_colors.first().copied().flatten();
        let secondary = theme_colors.get(1).copied().flatten();
        validate_color_opt("theme_colors", primary)?;
        validate_color_opt("theme_colors", secondary)?;
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
            state
                .dispatch_user(events::USER_UPDATE, &current, user_id)
                .await;
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
