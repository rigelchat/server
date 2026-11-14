use bcrypt::{DEFAULT_COST, hash, verify};
use chrono::Utc;

use crate::AppState;
use crate::constants::flags::UserFlags;
use crate::error::AppError;
use crate::models::rest::auth::{LoginRequest, LoginResponse, RegisterRequest};
use crate::models::user::{PublicUser, UserRow};
use crate::utils::{snowflake::DISCORD_SNOWFLAKE, token};

pub async fn login(state: &AppState, payload: LoginRequest) -> Result<LoginResponse, AppError> {
    let identifier = payload
        .identifier()
        .ok_or_else(|| AppError::BadRequest("Email or username is required".to_string()))?;

    let user = sqlx::query_as::<_, UserRow>(
        "SELECT * FROM users WHERE LOWER(email) = LOWER(?) OR LOWER(username) = LOWER(?) LIMIT 1",
    )
    .bind(identifier)
    .bind(identifier)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::Unauthorized("Invalid credentials".to_string()))?;

    let password_hash = user
        .password_hash
        .as_deref()
        .ok_or_else(|| AppError::Unauthorized("Invalid credentials".to_string()))?;

    if !verify(&payload.password, password_hash).unwrap_or(false) {
        return Err(AppError::Unauthorized("Invalid credentials".to_string()));
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

    Ok(LoginResponse {
        token: signed.token,
        user_id: user.id.clone(),
        user: PublicUser {
            id: user.id,
            username: user.username,
            discriminator: user.discriminator,
            global_name: user.global_name,
            avatar: user.avatar,
            banner: user.banner,
            bot: user.bot,
            flags: user.public_flags,
        },
    })
}

pub async fn register(state: &AppState, payload: RegisterRequest) -> Result<LoginResponse, AppError> {
    let email = payload.email.trim().to_lowercase();
    let username = payload.username.trim().to_string();

    if email.is_empty() || email.len() > 255 {
        return Err(AppError::BadRequest("Invalid email address".to_string()));
    }
    if username.is_empty() || username.len() > 32 {
        return Err(AppError::BadRequest(
            "Username must be between 1 and 32 characters".to_string(),
        ));
    }
    if payload.password.is_empty() {
        return Err(AppError::BadRequest("Password cannot be empty".to_string()));
    }

    let global_name = payload
        .global_name
        .map(|g| g.trim().to_string())
        .filter(|g| !g.is_empty());

    if let Some(ref g) = global_name {
        if g.len() > 32 {
            return Err(AppError::BadRequest(
                "Display name must be at most 32 characters".to_string(),
            ));
        }
    }

    let password_hash = hash(&payload.password, DEFAULT_COST)
        .map_err(|err| AppError::Internal(format!("Password hashing failed: {err}")))?;

    let mut tx = state.db.begin().await?;

    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM users WHERE LOWER(email) = LOWER(?) OR LOWER(username) = LOWER(?) LIMIT 1",
    )
    .bind(&email)
    .bind(&username)
    .fetch_optional(&mut *tx)
    .await?;

    if existing.is_some() {
        return Err(AppError::Conflict("Email or username already registered".to_string()));
    }

    // First non-bot account automatically becomes instance staff so it can bootstrap guilds.
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
    let discriminator = "0".to_string();

    sqlx::query(
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
    .bind(&email)
    .bind(&password_hash)
    .bind(&global_name)
    .bind(&discriminator)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO user_settings (id) VALUES (?)")
        .bind(&user_id)
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

    Ok(LoginResponse {
        token: signed.token,
        user_id: user_id.clone(),
        user: PublicUser {
            id: user_id,
            username,
            discriminator,
            global_name,
            avatar: None,
            banner: None,
            bot: false,
            flags: public_flags,
        },
    })
}