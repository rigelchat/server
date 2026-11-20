use std::collections::HashMap;

use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::events;
use crate::constants::limits::{MAX_GUILDS_PER_USER, MAX_MEMBERS_PAGE_LIMIT};
use crate::error::{AppError, ValidationCode};
use crate::models::guild::DiscordGuildCreate;
use crate::models::guild_member::{GuildMemberResponse, GuildMemberRole};
use crate::models::user::PartialUser;
use crate::services::guild_service;

#[derive(FromRow)]
struct MemberJoinRow {
    user_id: String,
    joined_at: u64,
    nick: Option<String>,
    username: String,
    discriminator: String,
    global_name: Option<String>,
    avatar: Option<String>,
    bot: bool,
    public_flags: u32,
}

#[derive(Serialize)]
struct GuildMemberAddEvent {
    guild_id: String,
    #[serde(flatten)]
    member: GuildMemberResponse,
}

pub async fn is_member(state: &AppState, guild_id: &str, user_id: &str) -> Result<bool, AppError> {
    let exists: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM guild_members WHERE guild_id = ? AND user_id = ? LIMIT 1",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?;

    Ok(exists.is_some())
}

pub async fn require_member(state: &AppState, guild_id: &str, user_id: &str) -> Result<(), AppError> {
    if !is_member(state, guild_id, user_id).await? {
        return Err(AppError::missing_permissions());
    }
    Ok(())
}

pub async fn add_member_to_guild(
    state: &AppState,
    user_id: &str,
    guild_id: &str,
) -> Result<(), AppError> {
    let guild_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?;

    if guild_exists.is_none() {
        return Err(AppError::unknown_guild());
    }

    let banned: Option<(String,)> =
        sqlx::query_as("SELECT id FROM guild_bans WHERE guild_id = ? AND user_id = ? LIMIT 1")
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;

    if banned.is_some() {
        return Err(AppError::banned_from_guild());
    }

    if is_member(state, guild_id, user_id).await? {
        return Ok(());
    }

    let joined_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&state.db)
            .await?;
    if joined_count >= MAX_GUILDS_PER_USER {
        return Err(AppError::max_guilds());
    }

    let now = Utc::now().timestamp_millis() as u64;
    sqlx::query("INSERT INTO guild_members (user_id, guild_id, joined_at) VALUES (?, ?, ?)")
        .bind(user_id)
        .bind(guild_id)
        .bind(now)
        .execute(&state.db)
        .await?;

    if let Ok(member) = fetch_member_response(state, guild_id, user_id).await {
        let evt = GuildMemberAddEvent {
            guild_id: guild_id.to_string(),
            member,
        };
        state
            .dispatch_guild(events::GUILD_MEMBER_ADD, &evt, guild_id)
            .await;
    }

    state.add_user_guild(user_id, guild_id).await;
    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;

    let gateway_guild = guild_service::build_full_gateway_guild(state, guild_id, user_id).await?;
    state
        .dispatch_user_dialect(events::GUILD_CREATE, user_id, |dialect| match dialect {
            ClientDialect::Fluxer => serde_json::to_value(&gateway_guild).unwrap_or_default(),
            ClientDialect::Discord => {
                serde_json::to_value(DiscordGuildCreate::from(gateway_guild.clone()))
                    .unwrap_or_default()
            }
        })
        .await;

    Ok(())
}

pub async fn leave_guild(state: &AppState, user_id: &str, guild_id: &str) -> Result<(), AppError> {
    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    if owner_id == user_id {
        return Err(AppError::validation(
            "guild_id",
            ValidationCode::CannotLeaveGuildAsOwner,
            "You can't leave a community as the owner.",
        ));
    }

    if !is_member(state, guild_id, user_id).await? {
        return Err(AppError::unknown_member());
    }

    let mut tx = state.db.begin().await?;

    sqlx::query("DELETE FROM guild_member_roles WHERE guild_id = ? AND user_id = ?")
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM guild_members WHERE guild_id = ? AND user_id = ?")
        .bind(guild_id)
        .bind(user_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    state.remove_user_guild(user_id, guild_id).await;
    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;

    state
        .dispatch_user(events::GUILD_DELETE, &json!({ "id": guild_id }), user_id)
        .await;

    state
        .dispatch_guild(
            events::GUILD_MEMBER_REMOVE,
            &json!({ "guild_id": guild_id, "user": { "id": user_id } }),
            guild_id,
        )
        .await;

    Ok(())
}

pub async fn get_guild_members(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    limit: Option<u32>,
    after: Option<&str>,
) -> Result<Vec<GuildMemberResponse>, AppError> {
    require_member(state, guild_id, user_id).await?;

    let limit = limit.unwrap_or(100).clamp(1, MAX_MEMBERS_PAGE_LIMIT);

    let mut qb = QueryBuilder::<MySql>::new(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = 
        "#,
    );
    qb.push_bind(guild_id);

    if let Some(after_id) = after {
        qb.push(" AND CAST(gm.user_id AS UNSIGNED) > CAST(");
        qb.push_bind(after_id);
        qb.push(" AS UNSIGNED)");
    }

    qb.push(" ORDER BY CAST(gm.user_id AS UNSIGNED) ASC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build_query_as::<MemberJoinRow>().fetch_all(&state.db).await?;

    let role_rows = sqlx::query_as::<_, GuildMemberRole>(
        "SELECT user_id, role_id FROM guild_member_roles WHERE guild_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    let mut roles_by_user: HashMap<String, Vec<String>> = HashMap::new();
    for r in role_rows {
        roles_by_user.entry(r.user_id).or_default().push(r.role_id);
    }

    Ok(rows
        .into_iter()
        .map(|m| {
            let roles = roles_by_user.remove(&m.user_id).unwrap_or_default();
            GuildMemberResponse {
                user: PartialUser {
                    id: m.user_id,
                    username: m.username,
                    discriminator: m.discriminator,
                    global_name: m.global_name,
                    avatar: m.avatar,
                    avatar_color: None,
                    bot: m.bot,
                    flags: m.public_flags,
                },
                nick: m.nick,
                avatar: None,
                banner: None,
                accent_color: None,
                roles,
                joined_at: m.joined_at,
                mute: false,
                deaf: false,
                communication_disabled_until: None,
            }
        })
        .collect())
}

pub async fn get_guild_member(
    state: &AppState,
    guild_id: &str,
    viewer_id: &str,
    target_user_id: &str,
) -> Result<GuildMemberResponse, AppError> {
    require_member(state, guild_id, viewer_id).await?;
    fetch_member_response(state, guild_id, target_user_id).await
}

async fn fetch_member_response(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<GuildMemberResponse, AppError> {
    let row = sqlx::query_as::<_, MemberJoinRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = ? AND gm.user_id = ?
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_member)?;

    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    Ok(GuildMemberResponse {
        user: PartialUser {
            id: row.user_id,
            username: row.username,
            discriminator: row.discriminator,
            global_name: row.global_name,
            avatar: row.avatar,
            avatar_color: None,
            bot: row.bot,
            flags: row.public_flags,
        },
        nick: row.nick,
        avatar: None,
        banner: None,
        accent_color: None,
        roles,
        joined_at: row.joined_at,
        mute: false,
        deaf: false,
        communication_disabled_until: None,
    })
}