use std::collections::HashSet;

use axum::http::StatusCode;
use chrono::Utc;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::error::{ApiErrorCode, AppError};
use crate::models::invite::{
    CreateInviteRequest, CreatedInvite, InviteChannel, InviteGuild, InviteResponse, InviteType,
};
use crate::models::serde_helpers::millis_to_iso;
use crate::services::{guild_service, user_service};

#[derive(FromRow)]
struct InviteDetailsRow {
    code: String,
    guild_id: String,
    channel_id: String,
    inviter_id: String,
    uses: u32,
    max_uses: u32,
    expires_at: Option<u64>,
    guild_name: String,
    guild_icon: Option<String>,
    guild_banner: Option<String>,
    guild_description: Option<String>,
    channel_name: String,
    channel_type: u32,
}

#[derive(FromRow)]
struct InviteDeleteRow {
    guild_id: String,
    inviter_id: String,
    owner_id: String,
}

async fn fetch_invite_details(
    state: &AppState,
    code: &str,
) -> Result<(InviteDetailsRow, InviteResponse), AppError> {
    let row = sqlx::query_as::<_, InviteDetailsRow>(
        r#"
        SELECT
            i.code, i.guild_id, i.channel_id, i.inviter_id,
            i.uses, i.max_uses, i.expires_at,
            g.name AS guild_name, g.icon AS guild_icon, g.banner AS guild_banner, g.description AS guild_description,
            c.name AS channel_name, c.type AS channel_type
        FROM invites i
        INNER JOIN guilds g ON g.id = i.guild_id
        INNER JOIN channels c ON c.id = i.channel_id
        WHERE i.code = ?
        "#,
    )
    .bind(code)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        AppError::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownInvite,
            "Unknown invite",
        )
    })?;

    let now = Utc::now().timestamp_millis() as u64;
    if row.expires_at.is_some_and(|exp| now > exp)
        || (row.max_uses > 0 && row.uses >= row.max_uses)
    {
        return Err(AppError::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownInvite,
            "Unknown invite",
        ));
    }

    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(&row.guild_id)
            .fetch_one(&state.db)
            .await?;

    let online_count = {
        let sessions = state.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.guilds.contains(&row.guild_id))
            .filter_map(|s| s.user_id.as_ref())
            .collect::<HashSet<_>>()
            .len()
    };

    let inviter = user_service::get_partial_user(state, &row.inviter_id).await.ok();

    let invite = InviteResponse {
        code: row.code.clone(),
        kind: InviteType::Guild,
        guild: InviteGuild {
            id: row.guild_id.clone(),
            name: row.guild_name.clone(),
            icon: row.guild_icon.clone(),
            banner: row.guild_banner.clone(),
            banner_width: None,
            banner_height: None,
            splash: None,
            splash_width: None,
            splash_height: None,
            splash_card_alignment: 0,
            embed_splash: None,
            embed_splash_width: None,
            embed_splash_height: None,
            features: vec![],
            badge_ids: vec![],
            description: row.guild_description.clone(),
        },
        channel: InviteChannel {
            id: row.channel_id.clone(),
            name: Some(row.channel_name.clone()),
            kind: row.channel_type,
        },
        inviter,
        member_count: Some(member_count),
        presence_count: Some(online_count),
        approximate_member_count: None,
        approximate_presence_count: None,
        expires_at: row.expires_at.map(millis_to_iso),
        temporary: false,
    };

    Ok((row, invite))
}

pub async fn create_channel_invite(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    payload: CreateInviteRequest,
) -> Result<CreatedInvite, AppError> {
    let guild_id = sqlx::query_scalar::<_, String>("SELECT guild_id FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::coded(
                StatusCode::NOT_FOUND,
                ApiErrorCode::UnknownChannel,
                "Unknown channel",
            )
        })?;

    guild_service::require_member(state, &guild_id, user_id).await?;

    let code = Uuid::new_v4().simple().to_string()[..8].to_string();
    let created_at = Utc::now().timestamp_millis() as u64;
    let max_uses = payload.max_uses.unwrap_or(0).min(100);
    let max_age = payload.max_age.unwrap_or(0).min(604800);
    let expires_at = (max_age > 0).then(|| created_at + (max_age as u64) * 1000);

    sqlx::query(
        "INSERT INTO invites (code, guild_id, channel_id, inviter_id, max_uses, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&code)
    .bind(&guild_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(max_uses)
    .bind(created_at)
    .bind(expires_at)
    .execute(&state.db)
    .await?;

    let (_, mut invite) = fetch_invite_details(state, &code).await?;
    invite.temporary = payload.temporary.unwrap_or(false);

    Ok(CreatedInvite {
        invite,
        created_at,
        uses: 0,
        max_uses,
        max_age,
    })
}

pub async fn get_invite(state: &AppState, code: &str) -> Result<InviteResponse, AppError> {
    let (_, invite) = fetch_invite_details(state, code).await?;
    Ok(invite)
}

pub async fn accept_invite(
    state: &AppState,
    code: &str,
    user_id: &str,
) -> Result<InviteResponse, AppError> {
    let (row, mut invite) = fetch_invite_details(state, code).await?;
    let already_member = guild_service::is_member(state, &row.guild_id, user_id).await?;

    if !already_member {
        guild_service::add_member_to_guild(state, user_id, &row.guild_id).await?;
        sqlx::query("UPDATE invites SET uses = uses + 1 WHERE code = ?")
            .bind(code)
            .execute(&state.db)
            .await?;

        if let Some(count) = invite.member_count.as_mut() {
            *count += 1;
        }
    }

    Ok(invite)
}

pub async fn delete_invite(
    state: &AppState,
    code: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, InviteDeleteRow>(
        r#"
        SELECT i.guild_id, i.inviter_id, g.owner_id
        FROM invites i
        INNER JOIN guilds g ON g.id = i.guild_id
        WHERE i.code = ?
        "#,
    )
    .bind(code)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        AppError::coded(
            StatusCode::NOT_FOUND,
            ApiErrorCode::UnknownInvite,
            "Unknown invite",
        )
    })?;

    if row.inviter_id != user_id && row.owner_id != user_id {
        guild_service::require_member(state, &row.guild_id, user_id).await?;

        let perms: Vec<u64> = sqlx::query_scalar(
            r#"
            SELECT r.permissions FROM roles r
            WHERE r.guild_id = ? AND (
                r.id = ?
                OR r.id IN (SELECT gmr.role_id FROM guild_member_roles gmr WHERE gmr.guild_id = ? AND gmr.user_id = ?)
            )
            "#,
        )
        .bind(&row.guild_id)
        .bind(&row.guild_id)
        .bind(&row.guild_id)
        .bind(user_id)
        .fetch_all(&state.db)
        .await?;

        let combined = perms.into_iter().fold(0u64, |acc, p| acc | p);
        let flags = PermissionFlags::from_bits_truncate(combined);
        if !flags.intersects(PermissionFlags::MANAGE_GUILD | PermissionFlags::ADMINISTRATOR) {
            return Err(AppError::coded(
                StatusCode::FORBIDDEN,
                ApiErrorCode::MissingPermissions,
                "You don't have the permissions required to perform this action.",
            ));
        }
    }

    sqlx::query("DELETE FROM invites WHERE code = ?")
        .bind(code)
        .execute(&state.db)
        .await?;

    Ok(())
}