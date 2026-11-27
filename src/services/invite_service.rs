use chrono::Utc;
use sqlx::FromRow;
use uuid::Uuid;

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::PermissionFlags;
use crate::constants::guild::FEATURE_DISCOVERABLE;
use crate::constants::invite::InviteType;
use crate::constants::limits::{
    INVITE_CODE_LENGTH, MAX_INVITE_AGE_SECONDS, MAX_INVITE_CODE_LENGTH, MAX_INVITE_USES,
    MAX_INVITES_PER_GUILD,
};
use crate::error::AppError;
use crate::models::invite::{InviteChannel, InviteGuild, InviteMetadata, InviteResponse};
use crate::models::rest::invites::CreateInviteRequest;
use crate::models::serde_helpers::millis_to_iso;
use crate::models::user::PartialUser;
use crate::services::{member_service, permission_service, thread};

const SELECT_INVITE_COLS: &str = r#"
    SELECT
        i.code, i.guild_id, i.channel_id,
        i.uses, i.max_uses, i.created_at, i.expires_at,
        g.name AS guild_name, g.icon AS guild_icon, g.banner AS guild_banner,
        g.description AS guild_description, g.discoverable AS guild_discoverable,
        c.name AS channel_name, c.type AS channel_type,
        u.id AS inviter_id, u.username AS inviter_username,
        u.discriminator AS inviter_discriminator, u.global_name AS inviter_global_name,
        u.avatar AS inviter_avatar, u.bot AS inviter_bot, u.public_flags AS inviter_public_flags
    FROM invites i
    INNER JOIN guilds g ON g.id = i.guild_id
    INNER JOIN channels c ON c.id = i.channel_id
    INNER JOIN users u ON u.id = i.inviter_id
"#;

#[derive(FromRow)]
struct InviteRow {
    code: String,
    guild_id: String,
    channel_id: String,
    uses: u32,
    max_uses: u32,
    created_at: u64,
    expires_at: Option<u64>,
    guild_name: String,
    guild_icon: Option<String>,
    guild_banner: Option<String>,
    guild_description: Option<String>,
    guild_discoverable: bool,
    channel_name: String,
    channel_type: u32,
    inviter_id: String,
    inviter_username: String,
    inviter_discriminator: String,
    inviter_global_name: Option<String>,
    inviter_avatar: Option<String>,
    inviter_bot: bool,
    inviter_public_flags: u32,
}

impl InviteRow {
    fn into_invite_response(self, member_count: i64, presence_count: usize) -> InviteResponse {
        InviteResponse {
            code: self.code,
            kind: InviteType::Guild,
            guild: InviteGuild {
                id: self.guild_id,
                name: self.guild_name,
                icon: self.guild_icon,
                banner: self.guild_banner,
                banner_width: None,
                banner_height: None,
                splash: None,
                splash_width: None,
                splash_height: None,
                splash_card_alignment: 0,
                embed_splash: None,
                embed_splash_width: None,
                embed_splash_height: None,
                features: if self.guild_discoverable {
                    vec![FEATURE_DISCOVERABLE.to_string()]
                } else {
                    vec![]
                },
                badge_ids: vec![],
                description: self.guild_description,
            },
            channel: InviteChannel {
                id: self.channel_id,
                name: Some(self.channel_name),
                kind: self.channel_type,
            },
            inviter: Some(PartialUser {
                id: self.inviter_id,
                username: self.inviter_username,
                discriminator: self.inviter_discriminator,
                global_name: self.inviter_global_name,
                avatar: self.inviter_avatar,
                avatar_color: None,
                bot: self.inviter_bot,
                flags: self.inviter_public_flags,
            }),
            member_count: Some(member_count),
            presence_count: Some(presence_count),
            expires_at: self.expires_at.map(millis_to_iso),
            temporary: false,
        }
    }

    fn into_invite_metadata(self, member_count: i64, presence_count: usize) -> InviteMetadata {
        let created_at = self.created_at;
        let uses = self.uses;
        let max_uses = self.max_uses;
        let max_age = compute_max_age(created_at, self.expires_at);
        InviteMetadata {
            invite: self.into_invite_response(member_count, presence_count),
            created_at,
            uses,
            max_uses,
            max_age,
        }
    }
}

#[derive(FromRow)]
struct InviteDeleteRow {
    guild_id: String,
    channel_id: String,
    inviter_id: String,
    owner_id: String,
}

fn compute_max_age(created_at: u64, expires_at: Option<u64>) -> u32 {
    let Some(exp) = expires_at else {
        return 0;
    };
    let secs = exp.saturating_sub(created_at) / 1000;
    u32::try_from(secs).unwrap_or(MAX_INVITE_AGE_SECONDS)
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

async fn hydrate_guild_invite_rows(
    state: &AppState,
    guild_id: &str,
    rows: Vec<InviteRow>,
) -> Result<Vec<InviteMetadata>, AppError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_one(&state.db)
            .await?;

    let presence_count = state.count_guild_online(guild_id).await as usize;

    Ok(rows
        .into_iter()
        .map(|row| row.into_invite_metadata(member_count, presence_count))
        .collect())
}

async fn fetch_invite_details(
    state: &AppState,
    code: &str,
) -> Result<(String, InviteResponse), AppError> {
    let row = sqlx::query_as::<_, InviteRow>(&format!("{SELECT_INVITE_COLS} WHERE i.code = ?"))
        .bind(code)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_invite)?;

    let now = Utc::now().timestamp_millis().max(0) as u64;
    if row.expires_at.is_some_and(|exp| now > exp) || (row.max_uses > 0 && row.uses >= row.max_uses)
    {
        return Err(AppError::unknown_invite());
    }

    let guild_id = row.guild_id.clone();
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(&guild_id)
            .fetch_one(&state.db)
            .await?;

    let online_count = state.count_guild_online(&guild_id).await as usize;
    let invite = row.into_invite_response(member_count, online_count);

    Ok((guild_id, invite))
}

pub async fn list_guild_invites(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<Vec<InviteMetadata>, AppError> {
    let perms = permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !perms.contains(PermissionFlags::MANAGE_GUILD) {
        return Err(AppError::missing_permissions());
    }

    let now = Utc::now().timestamp_millis().max(0) as u64;
    let mut rows = sqlx::query_as::<_, InviteRow>(&format!(
        r#"
        {SELECT_INVITE_COLS}
        WHERE i.guild_id = ?
          AND (i.expires_at IS NULL OR i.expires_at >= ?)
          AND (i.max_uses = 0 OR i.uses < i.max_uses)
        ORDER BY i.created_at DESC, i.code ASC
        "#
    ))
    .bind(guild_id)
    .bind(now)
    .fetch_all(&state.db)
    .await?;

    if !can_use_threads {
        rows.retain(|row| !ChannelType::requires_channel_threads_capability(row.channel_type));
    }

    hydrate_guild_invite_rows(state, guild_id, rows).await
}

pub async fn list_channel_invites(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<Vec<InviteMetadata>, AppError> {
    let (guild_id, channel_type): (String, u32) =
        sqlx::query_as("SELECT guild_id, type FROM channels WHERE id = ?")
            .bind(channel_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(AppError::unknown_channel)?;

    if !can_use_threads && ChannelType::requires_channel_threads_capability(channel_type) {
        return Err(AppError::unknown_channel());
    }

    if ChannelType::is_thread(channel_type) {
        let ctx = thread::require_thread_message_access(state, channel_id, user_id).await?;
        if !ctx.parent_perms.contains(PermissionFlags::MANAGE_CHANNELS) {
            return Err(AppError::missing_permissions());
        }
        return Err(AppError::invalid_channel_type());
    }

    permission_service::require_channel_permission(
        state,
        &guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_CHANNELS,
    )
    .await?;

    let now = Utc::now().timestamp_millis().max(0) as u64;
    let rows = sqlx::query_as::<_, InviteRow>(&format!(
        r#"
        {SELECT_INVITE_COLS}
        WHERE i.channel_id = ?
          AND (i.expires_at IS NULL OR i.expires_at >= ?)
          AND (i.max_uses = 0 OR i.uses < i.max_uses)
        ORDER BY i.created_at DESC, i.code ASC
        "#
    ))
    .bind(channel_id)
    .bind(now)
    .fetch_all(&state.db)
    .await?;

    hydrate_guild_invite_rows(state, &guild_id, rows).await
}

pub async fn create_channel_invite(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    payload: CreateInviteRequest,
) -> Result<InviteMetadata, AppError> {
    let (guild_id, channel_type): (String, u32) =
        sqlx::query_as("SELECT guild_id, type FROM channels WHERE id = ?")
            .bind(channel_id)
            .fetch_optional(&state.db)
            .await?
            .ok_or_else(AppError::unknown_channel)?;

    if !can_use_threads && ChannelType::requires_channel_threads_capability(channel_type) {
        return Err(AppError::unknown_channel());
    }

    if channel_type == ChannelType::GuildCategory as u32
        || channel_type == ChannelType::GuildLink as u32
        || ChannelType::is_thread(channel_type)
        || ChannelType::is_forum_like(channel_type)
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

    let created_at = Utc::now().timestamp_millis().max(0) as u64;

    sqlx::query(
        r#"
        DELETE FROM invites
        WHERE guild_id = ?
          AND ((expires_at IS NOT NULL AND expires_at < ?) OR (max_uses > 0 AND uses >= max_uses))
        "#,
    )
    .bind(&guild_id)
    .bind(created_at)
    .execute(&mut *tx)
    .await?;

    let invite_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invites WHERE guild_id = ?")
        .bind(&guild_id)
        .fetch_one(&mut *tx)
        .await?;
    if invite_count as u64 >= MAX_INVITES_PER_GUILD {
        return Err(AppError::max_invites());
    }

    let code = Uuid::new_v4().simple().to_string()[..INVITE_CODE_LENGTH].to_string();
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

    Ok(InviteMetadata {
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
    let (guild_id, mut invite) = fetch_invite_details(state, code).await?;

    let mut tx = state.db.begin().await?;
    let now = Utc::now().timestamp_millis().max(0) as u64;
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

    let inserted = member_service::insert_member_tx(&mut tx, user_id, &guild_id).await?;
    if !inserted {
        tx.rollback().await?;
        return Ok(invite);
    }

    sqlx::query("DELETE FROM invites WHERE code = ? AND max_uses > 0 AND uses >= max_uses")
        .bind(code)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    member_service::on_member_joined(state, user_id, &guild_id).await?;
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

    #[test]
    fn computes_max_age_from_timestamps() {
        let created = 1_700_000_000_000u64;
        assert_eq!(compute_max_age(created, None), 0);
        assert_eq!(compute_max_age(created, Some(created + 86_400_000)), 86_400);
        assert_eq!(compute_max_age(created, Some(created - 1_000)), 0);
        assert_eq!(
            compute_max_age(created, Some(u64::MAX)),
            MAX_INVITE_AGE_SECONDS
        );
    }
}
