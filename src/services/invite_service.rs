use std::collections::HashSet;

use chrono::Utc;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::error::AppError;
use crate::models::invite::{
    AcceptedInvite, CreatedInvite, InviteChannel, InviteGuild, InviteResponse, InviteType,
    InviteUserRef,
};
use crate::services::guild_service;

#[derive(FromRow)]
struct InviteDetailsRow {
    code: String,
    guild_id: String,
    channel_id: String,
    guild_name: String,
    guild_icon: Option<String>,
    guild_description: Option<String>,
    channel_name: String,
    channel_type: u32,
}

#[derive(FromRow)]
struct InviteAcceptRow {
    guild_id: String,
    channel_id: String,
    uses: u32,
    max_uses: u32,
    expires_at: Option<u64>,
}

pub async fn create_channel_invite(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> Result<CreatedInvite, AppError> {
    let guild_id = sqlx::query_scalar::<_, String>("SELECT guild_id FROM channels WHERE id = ?")
        .bind(channel_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound("Channel not found".to_string()))?;

    guild_service::require_member(state, &guild_id, user_id).await?;

    let code = Uuid::new_v4().simple().to_string()[..8].to_string();
    let created_at = Utc::now().timestamp_millis() as u64;

    sqlx::query(
        "INSERT INTO invites (code, guild_id, channel_id, inviter_id, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&code)
    .bind(&guild_id)
    .bind(channel_id)
    .bind(user_id)
    .bind(created_at)
    .execute(&state.db)
    .await?;

    Ok(CreatedInvite {
        code,
        guild_id,
        channel_id: channel_id.to_string(),
        inviter: InviteUserRef {
            id: user_id.to_string(),
        },
        uses: 0,
        max_uses: 0,
        created_at,
    })
}

pub async fn get_invite(state: &AppState, code: &str) -> Result<InviteResponse, AppError> {
    let row = sqlx::query_as::<_, InviteDetailsRow>(
        r#"
        SELECT
            i.code, i.guild_id, i.channel_id,
            g.name AS guild_name, g.icon AS guild_icon, g.description AS guild_description,
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
    .ok_or_else(|| AppError::NotFound("Unknown invite".to_string()))?;

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

    Ok(InviteResponse {
        code: row.code,
        kind: InviteType::Guild,
        guild: InviteGuild {
            id: row.guild_id,
            name: row.guild_name,
            icon: row.guild_icon,
            description: row.guild_description,
            features: vec![],
            verification_level: 0,
        },
        channel: InviteChannel {
            id: row.channel_id,
            name: row.channel_name,
            kind: row.channel_type,
        },
        approximate_member_count: member_count,
        approximate_presence_count: online_count,
    })
}

pub async fn accept_invite(
    state: &AppState,
    code: &str,
    user_id: &str,
) -> Result<AcceptedInvite, AppError> {
    let invite = sqlx::query_as::<_, InviteAcceptRow>(
        "SELECT guild_id, channel_id, uses, max_uses, expires_at FROM invites WHERE code = ?",
    )
    .bind(code)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| AppError::NotFound("Unknown invite".to_string()))?;

    let now = Utc::now().timestamp_millis() as u64;
    if let Some(expires_at) = invite.expires_at {
        if now > expires_at {
            return Err(AppError::NotFound("Invite has expired".to_string()));
        }
    }
    if invite.max_uses > 0 && invite.uses >= invite.max_uses {
        return Err(AppError::NotFound("Invite has reached max uses".to_string()));
    }

    guild_service::add_member_to_guild(state, user_id, &invite.guild_id).await?;

    sqlx::query("UPDATE invites SET uses = uses + 1 WHERE code = ?")
        .bind(code)
        .execute(&state.db)
        .await?;

    Ok(AcceptedInvite {
        guild_id: invite.guild_id,
        channel_id: invite.channel_id,
    })
}