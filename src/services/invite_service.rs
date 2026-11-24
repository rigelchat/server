use chrono::Utc;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::limits::{
    INVITE_CODE_LENGTH, MAX_INVITE_AGE_SECONDS, MAX_INVITE_CODE_LENGTH, MAX_INVITE_USES,
    MAX_INVITES_PER_GUILD,
};
use crate::error::AppError;
use crate::models::channel::ChannelType;
use crate::models::invite::{
    CreatedInvite, InviteChannel, InviteGuild, InviteResponse, InviteType,
};
use crate::models::rest::invites::CreateInviteRequest;
use crate::models::serde_helpers::millis_to_iso;
use crate::services::{member_service, permission_service, user};

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
    channel_id: String,
    inviter_id: String,
    owner_id: String,
}

fn validate_invite_code(code: &str) -> Result<(), AppError> {
    if code.is_empty()
        || code.len() > MAX_INVITE_CODE_LENGTH
        || !code.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return Err(AppError::unknown_invite());
    }
    Ok(())
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
    .ok_or_else(AppError::unknown_invite)?;

    let now = Utc::now().timestamp_millis() as u64;
    if row.expires_at.is_some_and(|exp| now > exp) || (row.max_uses > 0 && row.uses >= row.max_uses)
    {
        return Err(AppError::unknown_invite());
    }

    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(&row.guild_id)
            .fetch_one(&state.db)
            .await?;

    let online_count = state.count_guild_online(&row.guild_id).await as usize;
    let inviter = user::get_partial_user(state, &row.inviter_id).await.ok();

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
    let (guild_id, channel_type): (String, u32) =
        sqlx::query_as("SELECT guild_id, type FROM channels WHERE id = ?")
            .bind(channel_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(AppError::unknown_channel)?;

    if channel_type == ChannelType::GuildCategory as u32
        || channel_type == ChannelType::GuildLink as u32
    {
        return Err(AppError::invalid_channel_type());
    }

    permission_service::require_channel_permission(
        state,
        &guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::CREATE_INSTANT_INVITE,
    )
    .await?;

    let mut tx = state.db.begin().await?;

    let _: Option<(String,)> = sqlx::query_as("SELECT id FROM guilds WHERE id = ? FOR UPDATE")
        .bind(&guild_id)
        .fetch_optional(&mut *tx)
        .await?;

    let invite_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invites WHERE guild_id = ?")
        .bind(&guild_id)
        .fetch_one(&mut *tx)
        .await?;
    if invite_count as u64 >= MAX_INVITES_PER_GUILD {
        return Err(AppError::max_invites());
    }

    let code = Uuid::new_v4().simple().to_string()[..INVITE_CODE_LENGTH].to_string();
    let created_at = Utc::now().timestamp_millis() as u64;
    let max_uses = payload.max_uses.unwrap_or(0).min(MAX_INVITE_USES);
    let max_age = payload.max_age.unwrap_or(0).min(MAX_INVITE_AGE_SECONDS);
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
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

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
    validate_invite_code(code)?;
    let (_, invite) = fetch_invite_details(state, code).await?;
    Ok(invite)
}

pub async fn accept_invite(
    state: &AppState,
    code: &str,
    user_id: &str,
) -> Result<InviteResponse, AppError> {
    validate_invite_code(code)?;
    let (row, mut invite) = fetch_invite_details(state, code).await?;

    let mut tx = state.db.begin().await?;
    let now = Utc::now().timestamp_millis() as u64;
    let updated = sqlx::query(
        r#"
        UPDATE invites
        SET uses = uses + 1
        WHERE code = ?
          AND (max_uses = 0 OR uses < max_uses)
          AND (expires_at IS NULL OR expires_at >= ?)
        "#,
    )
    .bind(code)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    if updated.rows_affected() == 0 {
        return Err(AppError::unknown_invite());
    }

    let inserted = member_service::insert_member_tx(&mut tx, user_id, &row.guild_id).await?;
    if !inserted {
        tx.rollback().await?;
        return Ok(invite);
    }

    tx.commit().await?;

    member_service::on_member_joined(state, user_id, &row.guild_id).await?;
    if let Some(count) = invite.member_count.as_mut() {
        *count += 1;
    }

    Ok(invite)
}

pub async fn delete_invite(state: &AppState, code: &str, user_id: &str) -> Result<(), AppError> {
    validate_invite_code(code)?;
    let row = sqlx::query_as::<_, InviteDeleteRow>(
        r#"
        SELECT i.guild_id, i.channel_id, i.inviter_id, g.owner_id
        FROM invites i
        INNER JOIN guilds g ON g.id = i.guild_id
        WHERE i.code = ?
        "#,
    )
    .bind(code)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_invite)?;

    if row.inviter_id != user_id && row.owner_id != user_id {
        member_service::require_member(state, &row.guild_id, user_id).await?;
        let guild_perms =
            permission_service::compute_guild_permissions(state, &row.guild_id, user_id).await?;

        if !guild_perms.intersects(PermissionFlags::MANAGE_GUILD | PermissionFlags::ADMINISTRATOR) {
            let ch_perms = permission_service::compute_user_channel_permissions(
                state,
                &row.guild_id,
                &row.channel_id,
                user_id,
            )
            .await?;
            if !ch_perms.contains(PermissionFlags::MANAGE_CHANNELS) {
                return Err(AppError::missing_permissions());
            }
        }
    }

    sqlx::query("DELETE FROM invites WHERE code = ?")
        .bind(code)
        .execute(&state.db)
        .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_invite_code_format() {
        assert!(validate_invite_code("a1b2c3d4").is_ok());
        assert!(validate_invite_code("").is_err());
        assert!(validate_invite_code("bad-code!").is_err());
        assert!(validate_invite_code(&"x".repeat(MAX_INVITE_CODE_LENGTH + 1)).is_err());
    }
}
