use axum::http::StatusCode;
use bcrypt::{DEFAULT_COST, hash, verify};
use chrono::Utc;
use uuid::Uuid;

use crate::AppState;
use crate::constants::flags::UserFlags;
use crate::error::{ApiErrorCode, AppError};
use crate::models::rest::auth::{LoginRequest, LoginResponse, RegisterRequest};
use crate::models::user::{PartialUser, UserRow};
use crate::utils::{snowflake::DISCORD_SNOWFLAKE, token};

const VALID_THEMES: [&str; 5] = ["dark", "dark_legacy", "coal", "light", "system"];

fn normalize_login_identifier(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some((name, tag)) = trimmed.rsplit_once('#') {
        if !name.is_empty() && tag.chars().all(|c| c.is_ascii_digit()) {
            return name.to_string();
        }
    }
    if let Some((local, _host)) = trimmed.split_once('@') {
        if !local.is_empty() {
            return local.to_string();
        }
    }
    trimmed.to_string()
}

fn validate_username(username: &str) -> Result<(), AppError> {
    if username.is_empty() || username.len() > 32 {
        return Err(AppError::validation(
            "username",
            "USERNAME_LENGTH_INVALID",
            "Username must be between 1 and 32 characters.",
        ));
    }
    if !username.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(AppError::validation(
            "username",
            "USERNAME_INVALID_CHARACTERS",
            "Username can only contain Latin letters, numbers, and underscores.",
        ));
    }

    let lower = username.to_ascii_lowercase();
    if lower == "everyone" || lower == "here" {
        return Err(AppError::validation(
            "username",
            "USERNAME_RESERVED_VALUE",
            "Username can't be \"everyone\" or \"here\".",
        ));
    }
    if lower.contains("fluxer") || lower.contains("system message") {
        return Err(AppError::validation(
            "username",
            "USERNAME_CANNOT_CONTAIN_RESERVED_TERMS",
            "Username can't contain reserved terms.",
        ));
    }

    Ok(())
}

pub fn suggest_username_from_display(global_name: &str) -> String {
    let sanitized: String = global_name
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    let trimmed = sanitized.trim_matches('_').to_string();
    let candidate = if trimmed.is_empty() {
        format!("user_{}", &Uuid::new_v4().simple().to_string()[..8])
    } else {
        trimmed.chars().take(28).collect()
    };

    if validate_username(&candidate).is_ok() {
        candidate
    } else {
        format!("user_{}", &Uuid::new_v4().simple().to_string()[..8])
    }
}

pub async fn is_session_active(
    state: &AppState,
    session_id: &str,
    user_id: &str,
) -> Result<bool, AppError> {
    let exists: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM user_sessions WHERE id = ? AND user_id = ? LIMIT 1",
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(exists.is_some())
}

pub async fn is_username_available(state: &AppState, username: &str) -> Result<bool, AppError> {
    let trimmed = username.trim();
    validate_username(trimmed)?;

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM users WHERE LOWER(username) = LOWER(?) LIMIT 1")
            .bind(trimmed)
            .fetch_optional(&state.db)
            .await?;

    Ok(existing.is_none())
}

pub async fn logout(state: &AppState, session_id: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM user_sessions WHERE id = ?")
        .bind(session_id)
        .execute(&state.db)
        .await?;

    Ok(())
}

pub async fn login(state: &AppState, payload: LoginRequest) -> Result<LoginResponse, AppError> {
    let raw_identifier = payload.identifier().ok_or_else(|| {
        AppError::validation(
            "login",
            "INVALID_LOGIN_OR_PASSWORD",
            "Invalid username or password.",
        )
    })?;
    let normalized = normalize_login_identifier(raw_identifier);

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT * FROM users WHERE LOWER(username) = LOWER(?) OR LOWER(username) = LOWER(?) OR (email IS NOT NULL AND LOWER(email) = LOWER(?)) LIMIT 1",
    )
    .bind(raw_identifier)
    .bind(&normalized)
    .bind(raw_identifier)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        AppError::validation(
            "login",
            "INVALID_LOGIN_OR_PASSWORD",
            "Invalid username or password.",
        )
    })?;

    let password_hash = user.password_hash.as_deref().ok_or_else(|| {
        AppError::validation(
            "login",
            "INVALID_LOGIN_OR_PASSWORD",
            "Invalid username or password.",
        )
    })?;

    if !verify(&payload.password, password_hash).unwrap_or(false) {
        return Err(AppError::validation(
            "login",
            "INVALID_LOGIN_OR_PASSWORD",
            "Invalid username or password.",
        ));
    }

    let mut tx = state.db.begin().await?;

    sqlx::query("INSERT IGNORE INTO user_settings (id) VALUES (?)")
        .bind(&user.id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT IGNORE INTO user_profiles (id) VALUES (?)")
        .bind(&user.id)
        .execute(&mut *tx)
        .await?;

    let signed = token::sign(&user.id, &state.config.auth_secret);

    sqlx::query("INSERT INTO user_sessions (id, user_id, created_at) VALUES (?, ?, ?)")
        .bind(&signed.session_id)
        .bind(&user.id)
        .bind(signed.created_at)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let partial = PartialUser::from(&user);
    if let Some(code) = payload.invite_code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        let _ = crate::services::invite_service::accept_invite(state, code, &partial.id).await;
    }

    Ok(LoginResponse {
        token: signed.token,
        user_id: user.id,
        user: partial,
    })
}

pub async fn register(state: &AppState, payload: RegisterRequest) -> Result<LoginResponse, AppError> {
    let global_name = payload
        .global_name
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());

    if let Some(ref g) = global_name {
        if g.len() > 32 {
            return Err(AppError::validation(
                "global_name",
                "GLOBAL_NAME_LENGTH_INVALID",
                "Display name must be at most 32 characters.",
            ));
        }
    }

    let explicit_username = payload
        .username.as_deref()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());

    let mut username = match explicit_username {
        Some(u) => {
            validate_username(&u)?;
            u
        }
        None => {
            let legacy_local = payload
                .email
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .and_then(|e| e.split_once('@').map(|(l, _)| l))
                .unwrap_or("");
            let seed = global_name.as_deref().unwrap_or(legacy_local);
            suggest_username_from_display(seed)
        }
    };

    let password = payload.password.as_deref().unwrap_or("");
    if password.is_empty() || password.len() > 256 {
        return Err(AppError::validation(
            "password",
            "PASSWORD_LENGTH_INVALID",
            "Password must be between 1 and 256 characters.",
        ));
    }

    let theme = payload
        .theme
        .as_deref()
        .filter(|t| VALID_THEMES.contains(t))
        .unwrap_or("dark");

    let password_hash = hash(password, DEFAULT_COST).map_err(|err| {
        AppError::coded(
            StatusCode::INTERNAL_SERVER_ERROR,
            ApiErrorCode::InternalServerError,
            format!("Password hashing failed: {err}"),
        )
    })?;

    let mut tx = state.db.begin().await?;

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM users WHERE LOWER(username) = LOWER(?) LIMIT 1")
            .bind(&username)
            .fetch_optional(&mut *tx)
            .await?;

    if existing.is_some() {
        if payload.username.as_deref().is_some_and(|u| !u.trim().is_empty()) {
            return Err(AppError::validation(
                "username",
                "USERNAME_ALREADY_TAKEN",
                "This username is already taken.",
            ));
        }
        let suffix = &Uuid::new_v4().simple().to_string()[..4];
        let base: String = username.chars().take(27).collect();
        username = format!("{base}_{suffix}");
    }

    // First non-bot account becomes instance staff so it can bootstrap guilds.
    let (user_count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE bot = FALSE")
        .fetch_one(&mut *tx)
        .await?;

    let public_flags = if user_count == 0 {
        UserFlags::STAFF.bits()
    } else {
        0
    };

    let user_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let created_at = Utc::now().timestamp_millis() as u64;
    let discriminator = "0000".to_string();

    sqlx::query(
        r#"
        INSERT INTO users
            (id, created_at, bot, public_flags, username, email, verified, password_hash, global_name, discriminator)
        VALUES (?, ?, FALSE, ?, ?, NULL, TRUE, ?, ?, ?)
        "#,
    )
    .bind(&user_id)
    .bind(created_at)
    .bind(public_flags)
    .bind(&username)
    .bind(&password_hash)
    .bind(&global_name)
    .bind(&discriminator)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO user_settings (id, theme) VALUES (?, ?)")
        .bind(&user_id)
        .bind(theme)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO user_profiles (id) VALUES (?)")
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;

    let signed = token::sign(&user_id, &state.config.auth_secret);

    sqlx::query("INSERT INTO user_sessions (id, user_id, created_at) VALUES (?, ?, ?)")
        .bind(&signed.session_id)
        .bind(&user_id)
        .bind(signed.created_at)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    if let Some(code) = payload.invite_code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        let _ = crate::services::invite_service::accept_invite(state, code, &user_id).await;
    }

    Ok(LoginResponse {
        token: signed.token,
        user_id: user_id.clone(),
        user: PartialUser {
            id: user_id,
            username,
            discriminator,
            global_name,
            avatar: None,
            avatar_color: None,
            bot: false,
            flags: public_flags,
        },
    })
}