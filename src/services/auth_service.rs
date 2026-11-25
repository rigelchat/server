use std::sync::LazyLock;

use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use chrono::Utc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::AppState;
use crate::config::{LoginMode, RegistrationMode, TagStyle};
use crate::constants::flags::UserFlags;
use crate::constants::limits::{
    MAX_GLOBAL_NAME_LENGTH, MAX_PASSWORD_LENGTH, MAX_USERNAME_LENGTH,
    MAX_USERNAME_SUGGESTION_SEED_LENGTH, MIN_PASSWORD_LENGTH, SESSION_TOUCH_INTERVAL_MS,
    SESSION_TTL_MS,
};
use crate::constants::user::{DEFAULT_DISCRIMINATOR, DEFAULT_THEME, VALID_THEMES};
use crate::error::{AppError, ValidationCode};
use crate::models::rest::auth::{LoginRequest, LoginResponse, RegisterRequest};
use crate::models::user::{PartialUser, UserRow};
use crate::utils::validation::{is_valid_email, reject_control_chars};
use crate::utils::{snowflake::DISCORD_SNOWFLAKE, token};

static DUMMY_PASSWORD_HASH: LazyLock<String> = LazyLock::new(|| {
    Argon2::default()
        .hash_password(b"rigel-dummy-timing-check")
        .map(|h| h.to_string())
        .unwrap_or_default()
});

static ARGON2_SEMAPHORE: LazyLock<Semaphore> = LazyLock::new(|| {
    let permits = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 16))
        .unwrap_or(4);
    Semaphore::new(permits)
});

fn normalize_login_identifier(raw: &str, login_mode: LoginMode) -> String {
    let trimmed = raw.trim();
    if let Some((name, tag)) = trimmed.rsplit_once('#')
        && !name.is_empty()
        && tag.chars().all(|c| c.is_ascii_digit())
    {
        return name.to_string();
    }
    if login_mode == LoginMode::Username
        && let Some((local, _host)) = trimmed.split_once('@')
        && !local.is_empty()
    {
        return local.to_string();
    }
    trimmed.to_string()
}

pub fn validate_password(password: &str) -> Result<(), AppError> {
    let len = password.chars().count();
    if !(MIN_PASSWORD_LENGTH..=MAX_PASSWORD_LENGTH).contains(&len) {
        return Err(AppError::validation(
            "password",
            ValidationCode::PasswordLengthInvalid,
            format!(
                "Password must be between {MIN_PASSWORD_LENGTH} and {MAX_PASSWORD_LENGTH} characters."
            ),
        ));
    }
    Ok(())
}

pub fn validate_username(username: &str) -> Result<(), AppError> {
    if username.is_empty() || username.len() > MAX_USERNAME_LENGTH {
        return Err(AppError::validation(
            "username",
            ValidationCode::UsernameLengthInvalid,
            "Username must be between 1 and 32 characters.",
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(AppError::validation(
            "username",
            ValidationCode::UsernameInvalidCharacters,
            "Username can only contain Latin letters, numbers, and underscores.",
        ));
    }

    let lower = username.to_ascii_lowercase();
    if lower == "everyone" || lower == "here" {
        return Err(AppError::validation(
            "username",
            ValidationCode::UsernameReservedValue,
            "Username can't be \"everyone\" or \"here\".",
        ));
    }
    if lower.contains("fluxer") || lower.contains("system message") {
        return Err(AppError::validation(
            "username",
            ValidationCode::UsernameCannotContainReservedTerms,
            "Username can't contain reserved terms.",
        ));
    }

    Ok(())
}

pub fn suggest_username_from_display(global_name: &str) -> String {
    let bounded: String = global_name
        .trim()
        .chars()
        .take(MAX_USERNAME_SUGGESTION_SEED_LENGTH)
        .collect();
    let sanitized: String = bounded
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
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
) -> Result<Option<bool>, AppError> {
    let row: Option<(u64, Option<u64>, bool)> = sqlx::query_as(
        r#"
        SELECT us.created_at, us.last_used_at, u.bot
        FROM user_sessions us
        INNER JOIN users u ON u.id = us.user_id
        WHERE us.id = ? AND us.user_id = ?
        LIMIT 1
        "#,
    )
    .bind(session_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    let Some((created_at, last_used_at, is_bot)) = row else {
        return Ok(None);
    };

    let last_active_ms = last_used_at.unwrap_or(created_at);
    let now_ms = Utc::now().timestamp_millis().max(0) as u64;
    let elapsed = now_ms.saturating_sub(last_active_ms);

    if elapsed > SESSION_TTL_MS {
        let _ = sqlx::query("DELETE FROM user_sessions WHERE id = ? AND user_id = ?")
            .bind(session_id)
            .bind(user_id)
            .execute(&state.db)
            .await;
        return Ok(None);
    }

    if elapsed > SESSION_TOUCH_INTERVAL_MS {
        let _ =
            sqlx::query("UPDATE user_sessions SET last_used_at = ? WHERE id = ? AND user_id = ?")
                .bind(now_ms)
                .bind(session_id)
                .bind(user_id)
                .execute(&state.db)
                .await;
    }

    Ok(Some(is_bot))
}

pub async fn is_username_available(state: &AppState, username: &str) -> Result<bool, AppError> {
    let trimmed = username.trim();
    validate_username(trimmed)?;

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM users WHERE username = ? LIMIT 1")
            .bind(trimmed)
            .fetch_optional(&state.db)
            .await?;

    Ok(existing.is_none())
}

pub async fn logout(state: &AppState, user_id: &str, session_id: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM user_sessions WHERE id = ?")
        .bind(session_id)
        .execute(&state.db)
        .await?;

    state.hub.close_auth_session(user_id, session_id);
    Ok(())
}

pub async fn login(state: &AppState, payload: LoginRequest) -> Result<LoginResponse, AppError> {
    if payload.password.is_empty() || payload.password.len() > MAX_PASSWORD_LENGTH {
        return Err(AppError::validation(
            "login",
            ValidationCode::InvalidLoginOrPassword,
            "Invalid username or password.",
        ));
    }

    let raw_identifier = payload.identifier().ok_or_else(|| {
        AppError::validation(
            "login",
            ValidationCode::InvalidLoginOrPassword,
            "Invalid username or password.",
        )
    })?;
    let normalized = normalize_login_identifier(raw_identifier, state.config.auth.login_mode);

    let user = match state.config.auth.login_mode {
        LoginMode::Email => {
            sqlx::query_as::<_, UserRow>(&format!(
                "SELECT {} FROM users WHERE email = ? OR username = ? ORDER BY (email = ?) DESC LIMIT 1",
                UserRow::SELECT_COLS
            ))
            .bind(raw_identifier)
            .bind(&normalized)
            .bind(raw_identifier)
            .fetch_optional(&state.db)
            .await?
        }
        LoginMode::Username => {
            sqlx::query_as::<_, UserRow>(&format!(
                "SELECT {} FROM users WHERE username = ? OR username = ? LIMIT 1",
                UserRow::SELECT_COLS
            ))
            .bind(raw_identifier)
            .bind(&normalized)
            .fetch_optional(&state.db)
            .await?
        }
    };

    let (user, hash_owned) = match user.and_then(|mut u| u.password_hash.take().map(|h| (u, h))) {
        Some(pair) => pair,
        None => {
            let password_owned = payload.password;
            let _permit = ARGON2_SEMAPHORE
                .acquire()
                .await
                .map_err(|e| AppError::internal(format!("Argon2 semaphore closed: {e}")))?;
            let _ = tokio::task::spawn_blocking(move || {
                if let Ok(parsed) = PasswordHash::new(&DUMMY_PASSWORD_HASH) {
                    let _ = Argon2::default().verify_password(password_owned.as_bytes(), &parsed);
                }
            })
            .await;
            return Err(AppError::validation(
                "login",
                ValidationCode::InvalidLoginOrPassword,
                "Invalid username or password.",
            ));
        }
    };

    let password_owned = payload.password;
    let valid = {
        let _permit = ARGON2_SEMAPHORE
            .acquire()
            .await
            .map_err(|e| AppError::internal(format!("Argon2 semaphore closed: {e}")))?;
        tokio::task::spawn_blocking(move || {
            PasswordHash::new(&hash_owned)
                .map(|parsed| {
                    Argon2::default()
                        .verify_password(password_owned.as_bytes(), &parsed)
                        .is_ok()
                })
                .unwrap_or(false)
        })
        .await
        .unwrap_or(false)
    };

    if !valid {
        return Err(AppError::validation(
            "login",
            ValidationCode::InvalidLoginOrPassword,
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

    let signed = token::sign(&user.id, &state.config.auth.secret).map_err(AppError::internal)?;

    sqlx::query("INSERT INTO user_sessions (id, user_id, created_at) VALUES (?, ?, ?)")
        .bind(&signed.session_id)
        .bind(&user.id)
        .bind(signed.created_at)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    let partial = PartialUser::from(&user);
    if let Some(code) = payload
        .invite_code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        let _ = crate::services::invite_service::accept_invite(state, code, &partial.id).await;
    }

    Ok(LoginResponse {
        token: signed.token,
        user_id: user.id,
        user: partial,
    })
}

pub async fn register(
    state: &AppState,
    payload: RegisterRequest,
) -> Result<LoginResponse, AppError> {
    if state.config.auth.registration_mode == RegistrationMode::Closed {
        return Err(AppError::registration_closed(
            "Registration is closed on this instance.",
        ));
    }

    let email_to_store: Option<String> = match state.config.auth.login_mode {
        LoginMode::Email => {
            let raw = payload.email.as_deref().map(str::trim).unwrap_or("");
            if raw.is_empty() {
                return Err(AppError::validation(
                    "email",
                    ValidationCode::EmailIsRequired,
                    "Email is required.",
                ));
            }
            if !is_valid_email(raw) {
                return Err(AppError::validation(
                    "email",
                    ValidationCode::InvalidEmailFormat,
                    "Invalid email address format.",
                ));
            }
            let lower = raw.to_ascii_lowercase();
            let exists: Option<(String,)> =
                sqlx::query_as("SELECT id FROM users WHERE email = ? LIMIT 1")
                    .bind(&lower)
                    .fetch_optional(&state.db)
                    .await?;
            if exists.is_some() {
                return Err(AppError::validation(
                    "email",
                    ValidationCode::EmailAlreadyInUse,
                    "Email is already in use.",
                ));
            }
            Some(lower)
        }
        LoginMode::Username => None,
    };

    let global_name = payload
        .global_name
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());

    if let Some(ref g) = global_name {
        if g.chars().count() > MAX_GLOBAL_NAME_LENGTH {
            return Err(AppError::validation(
                "global_name",
                ValidationCode::GlobalNameLengthInvalid,
                "Display name must be at most 32 characters.",
            ));
        }
        reject_control_chars("global_name", g)?;
    }

    let explicit_username = payload
        .username
        .as_deref()
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());

    let has_explicit_username = explicit_username.is_some();
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
    validate_password(password)?;

    let theme = payload
        .theme
        .as_deref()
        .filter(|t| VALID_THEMES.contains(t))
        .unwrap_or(DEFAULT_THEME);

    let password_owned = password.to_string();
    let password_hash = {
        let _permit = ARGON2_SEMAPHORE
            .acquire()
            .await
            .map_err(|e| AppError::internal(format!("Argon2 semaphore closed: {e}")))?;
        tokio::task::spawn_blocking(move || {
            Argon2::default()
                .hash_password(password_owned.as_bytes())
                .map(|h| h.to_string())
                .map_err(|err| format!("Password hashing failed: {err}"))
        })
        .await
        .map_err(|err| AppError::internal(format!("Password hashing task failed: {err}")))?
        .map_err(AppError::internal)?
    };

    let mut tx = state.db.begin().await?;

    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM users WHERE username = ? LIMIT 1")
            .bind(&username)
            .fetch_optional(&mut *tx)
            .await?;

    if existing.is_some() {
        if has_explicit_username {
            return Err(AppError::validation(
                "username",
                ValidationCode::UsernameAlreadyTaken,
                "This username is already taken.",
            ));
        }
        let base: String = username.chars().take(27).collect();
        for _ in 0..5 {
            let suffix = &Uuid::new_v4().simple().to_string()[..4];
            let candidate = format!("{base}_{suffix}");
            let taken: Option<(String,)> =
                sqlx::query_as("SELECT id FROM users WHERE username = ? LIMIT 1")
                    .bind(&candidate)
                    .fetch_optional(&mut *tx)
                    .await?;
            username = candidate;
            if taken.is_none() {
                break;
            }
        }
    }

    let any_human: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM users WHERE bot = FALSE LIMIT 1")
        .fetch_optional(&mut *tx)
        .await?;

    let public_flags = if any_human.is_none() {
        let lock_row: Option<(String,)> = sqlx::query_as(
            "SELECT name FROM instance_locks WHERE name = 'bootstrap_staff' FOR UPDATE",
        )
        .fetch_optional(&mut *tx)
        .await?;
        if lock_row.is_none() {
            sqlx::query("INSERT IGNORE INTO instance_locks (name) VALUES ('bootstrap_staff')")
                .execute(&mut *tx)
                .await?;
            let _: Option<(String,)> = sqlx::query_as(
                "SELECT name FROM instance_locks WHERE name = 'bootstrap_staff' FOR UPDATE",
            )
            .fetch_optional(&mut *tx)
            .await?;
        }

        let recheck: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM users WHERE bot = FALSE LIMIT 1 FOR UPDATE")
                .fetch_optional(&mut *tx)
                .await?;
        if recheck.is_none() {
            UserFlags::STAFF.bits()
        } else {
            0
        }
    } else {
        0
    };

    let user_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let created_at = Utc::now().timestamp_millis() as u64;
    let discriminator = match state.config.auth.tag_style {
        TagStyle::None => DEFAULT_DISCRIMINATOR.to_string(),
        TagStyle::Random => {
            let bytes = Uuid::new_v4();
            let raw = u32::from_le_bytes([
                bytes.as_bytes()[0],
                bytes.as_bytes()[1],
                bytes.as_bytes()[2],
                bytes.as_bytes()[3],
            ]);
            format!("{:04}", (raw % 9999) + 1)
        }
    };

    let max_insert_attempts = if has_explicit_username { 1 } else { 4 };
    let base_for_retry: String = username.chars().take(27).collect();
    for attempt in 0..max_insert_attempts {
        let insert_res = sqlx::query(
            r#"
            INSERT INTO users
                (id, created_at, bot, public_flags, username, email, verified, password_hash, global_name, discriminator)
            VALUES (?, ?, FALSE, ?, ?, ?, TRUE, ?, ?, ?)
            "#,
        )
        .bind(&user_id)
        .bind(created_at)
        .bind(public_flags)
        .bind(&username)
        .bind(&email_to_store)
        .bind(&password_hash)
        .bind(&global_name)
        .bind(&discriminator)
        .execute(&mut *tx)
        .await;

        match insert_res {
            Ok(_) => break,
            Err(err) => {
                if let sqlx::Error::Database(db_err) = &err
                    && db_err.is_unique_violation()
                {
                    let msg = db_err.message();
                    if msg.contains("'email'") || msg.contains(".email'") {
                        return Err(AppError::validation(
                            "email",
                            ValidationCode::EmailAlreadyInUse,
                            "Email is already in use.",
                        ));
                    }
                    if !has_explicit_username && attempt + 1 < max_insert_attempts {
                        let suffix = &Uuid::new_v4().simple().to_string()[..4];
                        username = format!("{base_for_retry}_{suffix}");
                        continue;
                    }
                    return Err(AppError::validation(
                        "username",
                        ValidationCode::UsernameAlreadyTaken,
                        "This username is already taken.",
                    ));
                }
                return Err(err.into());
            }
        }
    }

    sqlx::query("INSERT INTO user_settings (id, theme) VALUES (?, ?)")
        .bind(&user_id)
        .bind(theme)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO user_profiles (id) VALUES (?)")
        .bind(&user_id)
        .execute(&mut *tx)
        .await?;

    let signed = token::sign(&user_id, &state.config.auth.secret).map_err(AppError::internal)?;

    sqlx::query("INSERT INTO user_sessions (id, user_id, created_at) VALUES (?, ?, ?)")
        .bind(&signed.session_id)
        .bind(&user_id)
        .bind(signed.created_at)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    if let Some(code) = payload
        .invite_code
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_username_rules() {
        assert!(validate_username("alice_01").is_ok());
        assert!(validate_username("").is_err());
        assert!(validate_username(&"a".repeat(MAX_USERNAME_LENGTH + 1)).is_err());
        assert!(validate_username("bad-name").is_err());
        assert!(validate_username("everyone").is_err());
        assert!(validate_username("HERE").is_err());
        assert!(validate_username("my_fluxer_bot").is_err());
        assert!(validate_username("system message").is_err());
    }

    #[test]
    fn validate_password_rules() {
        assert!(validate_password("12345678").is_ok());
        assert!(validate_password("").is_err());
        assert!(validate_password("1234567").is_err());
        assert!(validate_password(&"a".repeat(MAX_PASSWORD_LENGTH + 1)).is_err());
    }

    #[test]
    fn normalize_login_identifier_respects_mode() {
        assert_eq!(
            normalize_login_identifier("alice#0000", LoginMode::Username),
            "alice"
        );
        assert_eq!(
            normalize_login_identifier("alice@inst.local", LoginMode::Username),
            "alice"
        );
        assert_eq!(
            normalize_login_identifier("alice@example.com", LoginMode::Email),
            "alice@example.com"
        );
    }

    #[test]
    fn suggest_username_sanitizes_input() {
        assert_eq!(suggest_username_from_display("Jean Dupont!"), "jean_dupont");
        assert!(suggest_username_from_display("!!!").starts_with("user_"));
    }
}
