use std::collections::HashMap;

use chrono::Utc;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::config::GuildCreationPolicy;
use crate::constants::dialect::ClientDialect;
use crate::constants::flags::{PermissionFlags, UserFlags};
use crate::constants::gateway::events;
use crate::constants::limits::{
    MAX_GUILD_NAME_LENGTH, MAX_GUILDS_PAGE_LIMIT, MAX_GUILDS_PER_USER,
};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::ChannelType;
use crate::models::guild::{BaseGuild, DiscordGuildCreate, GatewayGuild, Guild, ListGuildsQuery};
use crate::models::guild_member::{
    BaseGuildMember, GatewayGuildMember, GuildMemberResponse, GuildMemberRole, GuildMemberUserRef,
};
use crate::models::permission::OverwriteType;
use crate::models::user::PartialUser;
use crate::services::{channel_service, member_service, permission_service, role_service};
use crate::utils::snowflake::DISCORD_SNOWFLAKE;

#[derive(FromRow)]
struct GatewayMemberRow {
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

pub async fn create_guild(state: &AppState, name: &str, owner_id: &str) -> Result<Guild, AppError> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_GUILD_NAME_LENGTH {
        return Err(AppError::validation(
            "name",
            ValidationCode::StringLengthInvalid,
            "String length must be between 1 and 100 characters.",
        ));
    }

    let (owner_flags,): (u32,) = sqlx::query_as("SELECT public_flags FROM users WHERE id = ?")
        .bind(owner_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unauthorized)?;

    if state.config.features.guild_creation_policy == GuildCreationPolicy::Staff
        && !UserFlags::from_bits_truncate(owner_flags).contains(UserFlags::STAFF)
    {
        return Err(AppError::missing_permissions_with(
            "You don't have permission to create communities on this instance.",
        ));
    }

    let joined_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE user_id = ?")
            .bind(owner_id)
            .fetch_one(&state.db)
            .await?;
    if joined_count >= MAX_GUILDS_PER_USER {
        return Err(AppError::max_guilds());
    }

    let now = Utc::now().timestamp_millis() as u64;
    let guild_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let text_category_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let voice_category_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let system_channel_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let afk_channel_id = DISCORD_SNOWFLAKE.generate(None).to_string();

    let default_permissions = PermissionFlags::DEFAULT_EVERYONE.bits();

    let channels = [
        (&text_category_id, ChannelType::GuildCategory, 0u32, None, "Text channels"),
        (&voice_category_id, ChannelType::GuildCategory, 1u32, None, "Voice channels"),
        (&system_channel_id, ChannelType::GuildText, 0u32, Some(&text_category_id), "general"),
        (&afk_channel_id, ChannelType::GuildVoice, 0u32, Some(&voice_category_id), "General"),
    ];

    let mut tx = state.db.begin().await?;

    sqlx::query(
        "INSERT INTO guilds (id, created_at, owner_id, name, system_channel_id) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&guild_id)
    .bind(now)
    .bind(owner_id)
    .bind(name)
    .bind(&system_channel_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO roles (id, guild_id, position, name, permissions) VALUES (?, ?, 0, '@everyone', ?)",
    )
    .bind(&guild_id)
    .bind(&guild_id)
    .bind(default_permissions)
    .execute(&mut *tx)
    .await?;

    for (id, kind, position, parent_id, ch_name) in channels {
        sqlx::query(
            "INSERT INTO channels (id, type, position, guild_id, parent_id, name) VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(kind as u32)
        .bind(position)
        .bind(&guild_id)
        .bind(parent_id)
        .bind(ch_name)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO channel_permission_overwrites (type, channel_id, target_id) VALUES (?, ?, ?)",
        )
        .bind(OverwriteType::Role as u32)
        .bind(id)
        .bind(&guild_id)
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("INSERT INTO guild_members (guild_id, user_id, joined_at) VALUES (?, ?, ?)")
        .bind(&guild_id)
        .bind(owner_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    state.add_user_guild(owner_id, &guild_id).await;

    let gateway_guild = build_full_gateway_guild(state, &guild_id, owner_id).await?;
    let props = gateway_guild.properties.clone();
    state
        .dispatch_user_dialect(events::GUILD_CREATE, owner_id, |dialect| match dialect {
            ClientDialect::Fluxer => serde_json::to_value(&gateway_guild).unwrap_or_default(),
            ClientDialect::Discord => {
                serde_json::to_value(DiscordGuildCreate::from(gateway_guild.clone()))
                    .unwrap_or_default()
            }
        })
        .await;

    Ok(props)
}

pub async fn get_guild(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<Guild, AppError> {
    let base = sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    member_service::require_member(state, guild_id, user_id).await?;

    let roles = role_service::fetch_guild_roles(state, guild_id).await?;
    let all_channels = channel_service::fetch_guild_channels(state, guild_id).await?;
    let channels =
        permission_service::filter_visible_channels(state, guild_id, user_id, all_channels).await?;
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_one(&state.db)
            .await?;

    let online_count = state.count_guild_online(guild_id).await;

    let mut guild = Guild::from(base);
    guild.roles = Some(roles);
    guild.emojis = Some(vec![]);
    guild.stickers = Some(vec![]);
    guild.channels = Some(channels);
    guild.member_count = Some(member_count.max(1) as u64);
    guild.online_count = Some(online_count);

    Ok(guild)
}

pub async fn get_user_guilds(
    state: &AppState,
    user_id: &str,
    query: ListGuildsQuery,
) -> Result<Vec<Guild>, AppError> {
    let limit = query
        .limit
        .unwrap_or(MAX_GUILDS_PAGE_LIMIT)
        .clamp(1, MAX_GUILDS_PAGE_LIMIT);
    let include_permissions = limit <= 100;
    let with_counts = query.with_counts.unwrap_or(false);

    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT g.* FROM guilds g INNER JOIN guild_members gm ON gm.guild_id = g.id WHERE gm.user_id = ",
    );
    qb.push_bind(user_id);

    if let Some(before) = &query.before {
        qb.push(" AND CAST(g.id AS UNSIGNED) < CAST(");
        qb.push_bind(before);
        qb.push(" AS UNSIGNED)");
    }
    if let Some(after) = &query.after {
        qb.push(" AND CAST(g.id AS UNSIGNED) > CAST(");
        qb.push_bind(after);
        qb.push(" AS UNSIGNED)");
    }

    qb.push(" ORDER BY CAST(g.id AS UNSIGNED) ASC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build_query_as::<BaseGuild>().fetch_all(&state.db).await?;
    if rows.is_empty() {
        return Ok(vec![]);
    }

    let mut perms_by_guild: HashMap<String, u64> = HashMap::new();
    if include_permissions {
        let non_owner_ids: Vec<&str> = rows
            .iter()
            .filter(|r| r.owner_id != user_id)
            .map(|r| r.id.as_str())
            .collect();

        if !non_owner_ids.is_empty() {
            let mut pqb = QueryBuilder::<MySql>::new(
                "SELECT r.guild_id, r.permissions FROM roles r LEFT JOIN guild_member_roles gmr ON gmr.role_id = r.id AND gmr.guild_id = r.guild_id AND gmr.user_id = ",
            );
            pqb.push_bind(user_id);
            pqb.push(" WHERE r.guild_id IN (");
            let mut sep = pqb.separated(", ");
            for gid in &non_owner_ids {
                sep.push_bind(*gid);
            }
            pqb.push(") AND (r.id = r.guild_id OR gmr.role_id IS NOT NULL)");

            let perm_rows: Vec<(String, u64)> = pqb.build_query_as().fetch_all(&state.db).await?;
            for (gid, p) in perm_rows {
                *perms_by_guild.entry(gid).or_insert(0) |= p;
            }
        }
    }

    let mut member_counts: HashMap<String, i64> = HashMap::new();
    if with_counts {
        let mut cqb = QueryBuilder::<MySql>::new(
            "SELECT guild_id, COUNT(*) AS cnt FROM guild_members WHERE guild_id IN (",
        );
        let mut sep = cqb.separated(", ");
        for r in &rows {
            sep.push_bind(&r.id);
        }
        cqb.push(") GROUP BY guild_id");

        let count_rows: Vec<(String, i64)> = cqb.build_query_as().fetch_all(&state.db).await?;
        for (gid, cnt) in count_rows {
            member_counts.insert(gid, cnt);
        }
    }

    let mut guilds = Vec::with_capacity(rows.len());
    for row in rows {
        let gid = row.id.clone();
        let is_owner = row.owner_id == user_id;
        let mut guild = Guild::from(row);

        if include_permissions {
            let perms = if is_owner {
                PermissionFlags::all().bits()
            } else {
                let raw = perms_by_guild.get(&gid).copied().unwrap_or(0);
                if PermissionFlags::from_bits_truncate(raw).contains(PermissionFlags::ADMINISTRATOR)
                {
                    PermissionFlags::all().bits()
                } else {
                    raw
                }
            };
            guild.permissions = Some(perms.to_string());
        }

        if with_counts {
            let cnt = member_counts.get(&gid).copied().unwrap_or(1).max(1) as u64;
            guild.approximate_member_count = Some(cnt);
            guild.approximate_presence_count = Some(state.count_guild_online(&gid).await);
        }

        guilds.push(guild);
    }

    Ok(guilds)
}

pub async fn build_gateway_guild(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<GatewayGuild, AppError> {
    let guild = sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let all_channels = channel_service::fetch_guild_channels(state, guild_id).await?;
    let channels =
        permission_service::filter_visible_channels(state, guild_id, user_id, all_channels).await?;
    let roles = role_service::fetch_guild_roles(state, guild_id).await?;

    let member = sqlx::query_as::<_, BaseGuildMember>(
        "SELECT joined_at, nick FROM guild_members WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;

    let member_roles: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ? AND role_id != ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_one(&state.db)
            .await?;

    let online_count = state.count_guild_online(guild_id).await;

    Ok(GatewayGuild {
        id: guild.id.clone(),
        joined_at: member.joined_at,
        member_count: member_count.max(1) as u64,
        online_count,
        channels,
        roles,
        members: vec![GatewayGuildMember {
            user: GuildMemberUserRef {
                id: user_id.to_string(),
            },
            roles: member_roles,
            nick: member.nick,
            avatar: None,
            banner: None,
            accent_color: None,
            joined_at: member.joined_at,
            mute: false,
            deaf: false,
            communication_disabled_until: None,
        }],
        emojis: vec![],
        stickers: vec![],
        presences: vec![],
        voice_states: vec![],
        threads: vec![],
        properties: Guild::from(guild),
    })
}

pub async fn build_full_gateway_guild(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<GatewayGuild<GuildMemberResponse>, AppError> {
    let guild = sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let all_channels = channel_service::fetch_guild_channels(state, guild_id).await?;
    let channels =
        permission_service::filter_visible_channels(state, guild_id, user_id, all_channels).await?;
    let roles = role_service::fetch_guild_roles(state, guild_id).await?;

    let rows = sqlx::query_as::<_, GatewayMemberRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = ?
        ORDER BY gm.joined_at ASC
        "#,
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

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

    let joined_at = rows.first().map(|r| r.joined_at).unwrap_or(guild.created_at);
    let members: Vec<GuildMemberResponse> = rows
        .into_iter()
        .map(|m| {
            let m_roles = roles_by_user.remove(&m.user_id).unwrap_or_default();
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
                roles: m_roles,
                joined_at: m.joined_at,
                mute: false,
                deaf: false,
                communication_disabled_until: None,
            }
        })
        .collect();

    let member_count = members.len().max(1) as u64;
    let online_count = state.count_guild_online(guild_id).await;

    Ok(GatewayGuild {
        id: guild.id.clone(),
        joined_at,
        member_count,
        online_count,
        channels,
        roles,
        members,
        emojis: vec![],
        stickers: vec![],
        presences: vec![],
        voice_states: vec![],
        threads: vec![],
        properties: Guild::from(guild),
    })
}