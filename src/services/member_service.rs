use std::collections::{HashMap, HashSet};

use chrono::Utc;
use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::{
    DEFAULT_MEMBERS_PAGE_LIMIT, MAX_GUILD_ROLES, MAX_GUILDS_PER_USER, MAX_MEMBERS_PAGE_LIMIT,
    MAX_NICK_LENGTH,
};
use crate::error::{AppError, ValidationCode};
use crate::models::gateway::events::{
    GuildDeleteEvent, GuildMemberAddEvent, GuildMemberRemoveEvent, GuildMemberRemoveUser,
    GuildMemberUpdateEvent,
};
use crate::models::guild_member::{GuildMemberResponse, GuildMemberUserRow};
use crate::models::rest::guilds::{UpdateCurrentMemberRequest, UpdateGuildMemberRequest};
use crate::services::{guild, permission_service, queries, role};

pub async fn is_member(state: &AppState, guild_id: &str, user_id: &str) -> Result<bool, AppError> {
    let exists: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM guild_members WHERE guild_id = ? AND user_id = ? LIMIT 1")
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;

    Ok(exists.is_some())
}

pub async fn require_member(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    if !is_member(state, guild_id, user_id).await? {
        let guild_exists: Option<(i32,)> =
            sqlx::query_as("SELECT 1 FROM guilds WHERE id = ? LIMIT 1")
                .bind(guild_id)
                .fetch_optional(&state.db)
                .await?;
        if guild_exists.is_none() {
            return Err(AppError::unknown_guild());
        }
        return Err(AppError::missing_permissions());
    }
    Ok(())
}

pub async fn insert_member_tx(
    conn: &mut sqlx::MySqlConnection,
    user_id: &str,
    guild_id: &str,
) -> Result<bool, AppError> {
    let guild_exists: Option<(String,)> = sqlx::query_as("SELECT id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *conn)
        .await?;

    if guild_exists.is_none() {
        return Err(AppError::unknown_guild());
    }

    let banned: Option<(String,)> =
        sqlx::query_as("SELECT id FROM guild_bans WHERE guild_id = ? AND user_id = ? LIMIT 1")
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;

    if banned.is_some() {
        return Err(AppError::banned_from_guild());
    }

    let already_member: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM guild_members WHERE guild_id = ? AND user_id = ? LIMIT 1")
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&mut *conn)
            .await?;

    if already_member.is_some() {
        return Ok(false);
    }

    let joined_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE user_id = ?")
            .bind(user_id)
            .fetch_one(&mut *conn)
            .await?;
    if joined_count >= MAX_GUILDS_PER_USER {
        return Err(AppError::max_guilds());
    }

    let now = Utc::now().timestamp_millis() as u64;
    let res =
        sqlx::query("INSERT INTO guild_members (user_id, guild_id, joined_at) VALUES (?, ?, ?)")
            .bind(user_id)
            .bind(guild_id)
            .bind(now)
            .execute(&mut *conn)
            .await;

    match res {
        Ok(_) => Ok(true),
        Err(sqlx::Error::Database(db_err)) if db_err.is_unique_violation() => Ok(false),
        Err(err) => Err(err.into()),
    }
}

pub async fn on_member_joined(
    state: &AppState,
    user_id: &str,
    guild_id: &str,
) -> Result<(), AppError> {
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

    let gateway_guild = guild::build_full_gateway_guild(state, guild_id, user_id).await?;
    state
        .dispatch_user(events::GUILD_CREATE, &gateway_guild, user_id)
        .await;

    Ok(())
}

pub async fn add_member_to_guild(
    state: &AppState,
    user_id: &str,
    guild_id: &str,
) -> Result<(), AppError> {
    let mut tx = state.db.begin().await?;
    let inserted = insert_member_tx(&mut tx, user_id, guild_id).await?;
    tx.commit().await?;

    if inserted {
        on_member_joined(state, user_id, guild_id).await?;
    }

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
        .dispatch_user(
            events::GUILD_DELETE,
            &GuildDeleteEvent { id: guild_id },
            user_id,
        )
        .await;

    state
        .dispatch_guild(
            events::GUILD_MEMBER_REMOVE,
            &GuildMemberRemoveEvent {
                guild_id,
                user: GuildMemberRemoveUser { id: user_id },
            },
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
    crate::utils::snowflake::validate_snowflake_opt("after", after)?;
    require_member(state, guild_id, user_id).await?;

    let limit = limit
        .unwrap_or(DEFAULT_MEMBERS_PAGE_LIMIT)
        .clamp(1, MAX_MEMBERS_PAGE_LIMIT);

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

    let rows = qb
        .build_query_as::<GuildMemberUserRow>()
        .fetch_all(&state.db)
        .await?;

    let mut roles_by_user = queries::fetch_guild_member_roles_map(&state.db, guild_id).await?;

    Ok(rows
        .into_iter()
        .map(|m| {
            let roles = roles_by_user.remove(&m.user_id).unwrap_or_default();
            m.into_member_response(roles)
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

pub async fn add_member_role(
    state: &AppState,
    guild_id: &str,
    actor_id: &str,
    target_user_id: &str,
    role_id: &str,
) -> Result<(), AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, actor_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    if role_id == guild_id {
        return Err(AppError::validation(
            "role_id",
            ValidationCode::InvalidRoleId,
            "Cannot assign the @everyone role.",
        ));
    }

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let target_exists: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM guild_members WHERE guild_id = ? AND user_id = ? FOR UPDATE")
            .bind(guild_id)
            .bind(target_user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if target_exists.is_none() {
        return Err(AppError::unknown_member());
    }

    let role_pos: Option<u32> =
        sqlx::query_scalar("SELECT position FROM roles WHERE id = ? AND guild_id = ?")
            .bind(role_id)
            .bind(guild_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(role_pos) = role_pos else {
        return Err(AppError::unknown_role());
    };

    if actor_id != owner_id {
        let highest_pos = role::highest_member_role_position(&mut *tx, guild_id, actor_id).await?;
        if highest_pos <= role_pos {
            return Err(AppError::missing_permissions());
        }
    }

    let res = sqlx::query(
        "INSERT IGNORE INTO guild_member_roles (guild_id, user_id, role_id) VALUES (?, ?, ?)",
    )
    .bind(guild_id)
    .bind(target_user_id)
    .bind(role_id)
    .execute(&mut *tx)
    .await?;

    let changed = res.rows_affected() > 0;
    let updated_member = if changed {
        Some(fetch_member_response_tx(&mut tx, guild_id, target_user_id).await?)
    } else {
        None
    };

    tx.commit().await?;

    if let Some(member) = updated_member {
        dispatch_member_update(state, guild_id, member).await;
    }

    Ok(())
}

pub async fn remove_member_role(
    state: &AppState,
    guild_id: &str,
    actor_id: &str,
    target_user_id: &str,
    role_id: &str,
) -> Result<(), AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, actor_id).await?;
    if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
        return Err(AppError::missing_permissions());
    }

    if role_id == guild_id {
        return Err(AppError::validation(
            "role_id",
            ValidationCode::InvalidRoleId,
            "Cannot remove the @everyone role.",
        ));
    }

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let target_exists: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM guild_members WHERE guild_id = ? AND user_id = ? FOR UPDATE")
            .bind(guild_id)
            .bind(target_user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if target_exists.is_none() {
        return Err(AppError::unknown_member());
    }

    let role_pos: Option<u32> =
        sqlx::query_scalar("SELECT position FROM roles WHERE id = ? AND guild_id = ?")
            .bind(role_id)
            .bind(guild_id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some(role_pos) = role_pos else {
        return Err(AppError::unknown_role());
    };

    if actor_id != owner_id {
        let highest_pos = role::highest_member_role_position(&mut *tx, guild_id, actor_id).await?;
        if highest_pos <= role_pos {
            return Err(AppError::missing_permissions());
        }
    }

    let res = sqlx::query(
        "DELETE FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id = ?",
    )
    .bind(guild_id)
    .bind(target_user_id)
    .bind(role_id)
    .execute(&mut *tx)
    .await?;

    let changed = res.rows_affected() > 0;
    let updated_member = if changed {
        Some(fetch_member_response_tx(&mut tx, guild_id, target_user_id).await?)
    } else {
        None
    };

    tx.commit().await?;

    if let Some(member) = updated_member {
        dispatch_member_update(state, guild_id, member).await;
    }

    Ok(())
}

pub async fn update_current_member(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: UpdateCurrentMemberRequest,
) -> Result<GuildMemberResponse, AppError> {
    let perms = permission_service::compute_guild_permissions(state, guild_id, user_id).await?;

    let mut changed = false;
    if let Some(nick_opt) = payload.nick {
        let next_nick = normalize_nick_opt(nick_opt.as_deref())?;
        let current_nick = sqlx::query_scalar::<_, Option<String>>(
            "SELECT nick FROM guild_members WHERE guild_id = ? AND user_id = ?",
        )
        .bind(guild_id)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await?
        .flatten();

        if current_nick != next_nick && perms.contains(PermissionFlags::CHANGE_NICKNAME) {
            sqlx::query("UPDATE guild_members SET nick = ? WHERE guild_id = ? AND user_id = ?")
                .bind(&next_nick)
                .bind(guild_id)
                .bind(user_id)
                .execute(&state.db)
                .await?;
            changed = true;
        }
    }

    let member = fetch_member_response(state, guild_id, user_id).await?;
    if changed {
        dispatch_member_update(state, guild_id, member.clone()).await;
    }
    Ok(member)
}

pub async fn update_member(
    state: &AppState,
    guild_id: &str,
    actor_id: &str,
    target_user_id: &str,
    payload: UpdateGuildMemberRequest,
) -> Result<GuildMemberResponse, AppError> {
    let actor_perms =
        permission_service::compute_guild_permissions(state, guild_id, actor_id).await?;

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let current_nick_row = sqlx::query_scalar::<_, Option<String>>(
        "SELECT nick FROM guild_members WHERE guild_id = ? AND user_id = ? FOR UPDATE",
    )
    .bind(guild_id)
    .bind(target_user_id)
    .fetch_optional(&mut *tx)
    .await?;

    let Some(current_nick) = current_nick_row else {
        return Err(AppError::unknown_member());
    };

    let mut changed = false;
    let is_owner = actor_id == owner_id;
    let mut cached_actor_highest: Option<u32> = None;

    if let Some(nick_opt) = payload.nick {
        let next_nick = normalize_nick_opt(nick_opt.as_deref())?;
        if target_user_id == actor_id {
            if current_nick != next_nick
                && (actor_perms.contains(PermissionFlags::CHANGE_NICKNAME)
                    || actor_perms.contains(PermissionFlags::MANAGE_NICKNAMES))
            {
                sqlx::query("UPDATE guild_members SET nick = ? WHERE guild_id = ? AND user_id = ?")
                    .bind(&next_nick)
                    .bind(guild_id)
                    .bind(target_user_id)
                    .execute(&mut *tx)
                    .await?;
                changed = true;
            }
        } else {
            if !actor_perms.contains(PermissionFlags::MANAGE_NICKNAMES) {
                return Err(AppError::missing_permissions());
            }
            if !is_owner {
                if target_user_id == owner_id {
                    return Err(AppError::missing_permissions());
                }
                let actor_highest =
                    role::highest_member_role_position(&mut *tx, guild_id, actor_id).await?;
                cached_actor_highest = Some(actor_highest);
                let target_highest =
                    role::highest_member_role_position(&mut *tx, guild_id, target_user_id).await?;
                if actor_highest <= target_highest {
                    return Err(AppError::missing_permissions());
                }
            }
            if current_nick != next_nick {
                sqlx::query("UPDATE guild_members SET nick = ? WHERE guild_id = ? AND user_id = ?")
                    .bind(&next_nick)
                    .bind(guild_id)
                    .bind(target_user_id)
                    .execute(&mut *tx)
                    .await?;
                changed = true;
            }
        }
    }

    if let Some(raw_roles) = payload.roles {
        if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
            return Err(AppError::missing_permissions());
        }
        if raw_roles.len() > MAX_GUILD_ROLES as usize {
            return Err(AppError::validation(
                "roles",
                ValidationCode::InvalidFormat,
                "Too many roles specified.",
            ));
        }
        if raw_roles.iter().any(|r| r == guild_id) {
            return Err(AppError::validation(
                "roles",
                ValidationCode::InvalidRoleId,
                "Cannot assign the @everyone role.",
            ));
        }

        let guild_roles = sqlx::query_as::<_, (String, u32)>(
            "SELECT id, position FROM roles WHERE guild_id = ? AND id != ?",
        )
        .bind(guild_id)
        .bind(guild_id)
        .fetch_all(&mut *tx)
        .await?;
        let role_pos_by_id: HashMap<String, u32> = guild_roles.into_iter().collect();

        let mut seen = HashSet::new();
        let mut next_roles = Vec::new();
        for rid in raw_roles {
            if role_pos_by_id.contains_key(&rid) && seen.insert(rid.clone()) {
                next_roles.push(rid);
            }
        }

        let current_roles: Vec<String> = sqlx::query_scalar(
            "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id != ?",
        )
        .bind(guild_id)
        .bind(target_user_id)
        .bind(guild_id)
        .fetch_all(&mut *tx)
        .await?;
        let current_set: HashSet<String> = current_roles.into_iter().collect();

        if seen != current_set {
            if !is_owner {
                let actor_highest = match cached_actor_highest {
                    Some(pos) => pos,
                    None => {
                        role::highest_member_role_position(&mut *tx, guild_id, actor_id).await?
                    }
                };
                for rid in seen.symmetric_difference(&current_set) {
                    let allowed = role_pos_by_id
                        .get(rid)
                        .is_some_and(|&pos| actor_highest > pos);
                    if !allowed {
                        return Err(AppError::missing_permissions());
                    }
                }
            }

            sqlx::query("DELETE FROM guild_member_roles WHERE guild_id = ? AND user_id = ?")
                .bind(guild_id)
                .bind(target_user_id)
                .execute(&mut *tx)
                .await?;

            for rid in &next_roles {
                sqlx::query(
                    "INSERT INTO guild_member_roles (guild_id, user_id, role_id) VALUES (?, ?, ?)",
                )
                .bind(guild_id)
                .bind(target_user_id)
                .bind(rid)
                .execute(&mut *tx)
                .await?;
            }
            changed = true;
        }
    }

    let member = fetch_member_response_tx(&mut tx, guild_id, target_user_id).await?;
    tx.commit().await?;

    if changed {
        dispatch_member_update(state, guild_id, member.clone()).await;
    }

    Ok(member)
}

async fn dispatch_member_update(state: &AppState, guild_id: &str, member: GuildMemberResponse) {
    let evt = GuildMemberUpdateEvent {
        guild_id: guild_id.to_string(),
        member,
    };
    state
        .dispatch_guild(events::GUILD_MEMBER_UPDATE, &evt, guild_id)
        .await;
    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, guild_id).await;
}

fn normalize_nick_opt(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MAX_NICK_LENGTH {
        return Err(AppError::validation(
            "nick",
            ValidationCode::StringLengthInvalid,
            "Nickname must be between 1 and 32 characters.",
        ));
    }
    crate::utils::validation::reject_control_chars("nick", trimmed)?;
    Ok(Some(trimmed.to_string()))
}

async fn fetch_member_response(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<GuildMemberResponse, AppError> {
    let mut conn = state.db.acquire().await?;
    fetch_member_response_tx(&mut conn, guild_id, user_id).await
}

async fn fetch_member_response_tx(
    conn: &mut sqlx::MySqlConnection,
    guild_id: &str,
    user_id: &str,
) -> Result<GuildMemberResponse, AppError> {
    let row = sqlx::query_as::<_, GuildMemberUserRow>(
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
    .fetch_optional(&mut *conn)
    .await?
    .ok_or_else(AppError::unknown_member)?;

    let roles = queries::fetch_member_roles(&mut *conn, guild_id, user_id).await?;

    Ok(row.into_member_response(roles))
}
