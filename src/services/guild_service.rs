use std::collections::{HashMap, HashSet};

use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::config::GuildCreationPolicy;
use crate::constants::limits::{MAX_GUILDS_PAGE_LIMIT, MAX_GUILDS_PER_USER, MAX_GUILD_NAME_LENGTH, MAX_MEMBERS_PAGE_LIMIT};
use crate::constants::flags::{PermissionFlags, UserFlags};
use crate::constants::gateway::events;
use crate::constants::guild::{DiscoveryCategory, VerificationLevel};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, ChannelType, GuildChannel, PermissionOverwrite,
};
use crate::models::guild::{BaseGuild, DiscordGuildCreate, GatewayGuild, Guild, ListGuildsQuery};
use crate::models::guild_member::{
    BaseGuildMember, GatewayGuildMember, GuildMemberResponse, GuildMemberRole, GuildMemberUserRef,
};
use crate::models::permission::OverwriteType;
use crate::models::rest::discoverable_guilds::{
    DiscoverableGuild as RestDiscoverableGuild, DiscoverableGuildsResponse,
};
use crate::models::rest::discovery::{
    CategoryCount, DiscoverableGuild, DiscoverySearchQuery, GuildDiscoveryResponse,
};
use crate::models::role::{Role, RoleRow};
use crate::models::user::PartialUser;
use crate::utils::snowflake::DISCORD_SNOWFLAKE;

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

#[derive(FromRow)]
struct DiscoverableGuildRow {
    id: String,
    name: String,
    icon: Option<String>,
    banner: Option<String>,
    description: Option<String>,
    member_count: i64,
}

#[derive(FromRow)]
struct GuildOnlineCountRow {
    guild_id: String,
    online_count: i64,
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

pub async fn create_guild(state: &AppState, name: &str, owner_id: &str) -> Result<Guild, AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > MAX_GUILD_NAME_LENGTH {
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
        .ok_or_else(|| {
            AppError::unauthorized()
        })?;

    if state.config.features.guild_creation_policy == GuildCreationPolicy::Staff
        && !UserFlags::from_bits_truncate(owner_flags).contains(UserFlags::STAFF)
    {
        return Err(AppError::missing_permissions_with("You don't have permission to create communities on this instance."));
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

    let gateway_guild = build_full_gateway_guild(state, &guild_id).await?;
    let props = gateway_guild.properties.clone();
    state
        .dispatch_user_dialect(events::GUILD_CREATE, owner_id, |dialect| match dialect {
            crate::constants::dialect::ClientDialect::Fluxer => {
                serde_json::to_value(&gateway_guild).unwrap_or_default()
            }
            crate::constants::dialect::ClientDialect::Discord => {
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
        .ok_or_else(|| {
            AppError::unknown_guild()
        })?;

    require_member(state, guild_id, user_id).await?;

    let roles = fetch_guild_roles(state, guild_id).await?;
    let channels = fetch_guild_channels(state, guild_id).await?;
    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_one(&state.db)
            .await?;

    let online_count = count_guild_online(state, guild_id).await;

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
    let limit = query.limit.unwrap_or(MAX_GUILDS_PAGE_LIMIT).clamp(1, MAX_GUILDS_PAGE_LIMIT);
    let include_permissions = limit <= 100;
    let with_counts = query.with_counts.unwrap_or(false);

    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT g.* FROM guilds g INNER JOIN guild_members gm ON gm.guild_id = g.id WHERE gm.user_id = ",
    );
    qb.push_bind(user_id);

    if let Some(before) = &query.before {
        qb.push(" AND g.id < ");
        qb.push_bind(before);
    }
    if let Some(after) = &query.after {
        qb.push(" AND g.id > ");
        qb.push_bind(after);
    }

    qb.push(" ORDER BY g.id ASC LIMIT ");
    qb.push_bind(limit);

    let rows = qb.build_query_as::<BaseGuild>().fetch_all(&state.db).await?;
    let mut guilds = Vec::with_capacity(rows.len());

    for row in rows {
        let gid = row.id.clone();
        let owner_id = row.owner_id.clone();
        let mut guild = Guild::from(row);

        if include_permissions {
            let perms = compute_guild_permissions(state, &gid, user_id, &owner_id).await?;
            guild.permissions = Some(perms.to_string());
        }

        if with_counts {
            let member_count: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
                    .bind(&gid)
                    .fetch_one(&state.db)
                    .await?;
            guild.approximate_member_count = Some(member_count.max(1) as u64);
            guild.approximate_presence_count = Some(count_guild_online(state, &gid).await);
        }

        guilds.push(guild);
    }

    Ok(guilds)
}

async fn compute_guild_permissions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    owner_id: &str,
) -> Result<u64, AppError> {
    if user_id == owner_id {
        return Ok(PermissionFlags::all().bits());
    }

    let role_perms: Vec<u64> = sqlx::query_scalar(
        r#"
        SELECT r.permissions FROM roles r
        WHERE r.guild_id = ? AND (
            r.id = ?
            OR r.id IN (SELECT gmr.role_id FROM guild_member_roles gmr WHERE gmr.guild_id = ? AND gmr.user_id = ?)
        )
        "#,
    )
    .bind(guild_id)
    .bind(guild_id)
    .bind(guild_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    let combined = role_perms.into_iter().fold(0u64, |acc, p| acc | p);
    if PermissionFlags::from_bits_truncate(combined).contains(PermissionFlags::ADMINISTRATOR) {
        return Ok(PermissionFlags::all().bits());
    }

    Ok(combined)
}

async fn count_guild_online(state: &AppState, guild_id: &str) -> u64 {
    let sessions = state.sessions.read().await;
    sessions
        .values()
        .filter(|s| s.guilds.contains(guild_id))
        .filter_map(|s| s.user_id.as_ref())
        .collect::<HashSet<_>>()
        .len() as u64
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

    let gateway_guild = build_full_gateway_guild(state, guild_id).await?;
    state
        .dispatch_user_dialect(events::GUILD_CREATE, user_id, |dialect| match dialect {
            crate::constants::dialect::ClientDialect::Fluxer => {
                serde_json::to_value(&gateway_guild).unwrap_or_default()
            }
            crate::constants::dialect::ClientDialect::Discord => {
                serde_json::to_value(DiscordGuildCreate::from(gateway_guild.clone()))
                    .unwrap_or_default()
            }
        })
        .await;

    Ok(())
}

pub async fn leave_guild(state: &AppState, user_id: &str, guild_id: &str) -> Result<(), AppError> {
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
            &json!({ "id": guild_id }),
            user_id,
        )
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

pub async fn get_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> Result<ChannelResponse, AppError> {
    let channel = sqlx::query_as::<_, GuildChannel>(
        r#"
        SELECT
            c.id, c.type, c.position, c.guild_id, c.parent_id, c.name,
            (SELECT m.id FROM messages m WHERE m.channel_id = c.id ORDER BY m.id DESC LIMIT 1) AS last_message_id
        FROM channels c
        WHERE c.id = ?
        "#,
    )
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| {
        AppError::unknown_channel()
    })?;

    require_member(state, &channel.guild_id, user_id).await?;

    let overwrites = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(PermissionOverwrite::from)
    .collect();

    Ok(ChannelResponse::from_parts(channel, overwrites))
}

pub async fn fetch_guild_channels(
    state: &AppState,
    guild_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    let channels = sqlx::query_as::<_, GuildChannel>(
        r#"
        SELECT
            c.id, c.type, c.position, c.guild_id, c.parent_id, c.name,
            (SELECT m.id FROM messages m WHERE m.channel_id = c.id ORDER BY m.id DESC LIMIT 1) AS last_message_id
        FROM channels c
        WHERE c.guild_id = ?
        ORDER BY c.position ASC
        "#,
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    if channels.is_empty() {
        return Ok(vec![]);
    }

    let overwrites = sqlx::query_as::<_, ChannelOverwriteRow>(
        r#"
        SELECT o.channel_id, o.type, o.target_id, o.allow, o.deny
        FROM channel_permission_overwrites o
        INNER JOIN channels c ON c.id = o.channel_id
        WHERE c.guild_id = ?
        "#,
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    let mut overwrites_by_channel: HashMap<String, Vec<PermissionOverwrite>> = HashMap::new();
    for ow in overwrites {
        overwrites_by_channel
            .entry(ow.channel_id.clone())
            .or_default()
            .push(PermissionOverwrite::from(ow));
    }

    Ok(channels
        .into_iter()
        .map(|base| {
            let permission_overwrites = overwrites_by_channel.remove(&base.id).unwrap_or_default();
            ChannelResponse::from_parts(base, permission_overwrites)
        })
        .collect())
}

pub async fn get_guild_channels(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    require_member(state, guild_id, user_id).await?;
    fetch_guild_channels(state, guild_id).await
}

pub async fn fetch_guild_roles(state: &AppState, guild_id: &str) -> Result<Vec<Role>, AppError> {
    let rows = sqlx::query_as::<_, RoleRow>(
        "SELECT id, position, name, color, unicode_emoji, hoist, mentionable, permissions FROM roles WHERE guild_id = ? ORDER BY position DESC",
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    Ok(rows.into_iter().map(Role::from).collect())
}

pub async fn get_guild_roles(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<Vec<Role>, AppError> {
    require_member(state, guild_id, user_id).await?;
    fetch_guild_roles(state, guild_id).await
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
        qb.push(" AND gm.user_id > ");
        qb.push_bind(after_id);
    }

    qb.push(" ORDER BY gm.user_id ASC LIMIT ");
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
    .ok_or_else(|| {
        AppError::unknown_member()
    })?;

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

pub async fn build_gateway_guild(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<GatewayGuild, AppError> {
    let guild = sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::unknown_guild()
        })?;

    let channels = fetch_guild_channels(state, guild_id).await?;
    let roles = fetch_guild_roles(state, guild_id).await?;

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

    let online_count = count_guild_online(state, guild_id).await.max(1);

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
) -> Result<GatewayGuild<GuildMemberResponse>, AppError> {
    let guild = sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| {
            AppError::unknown_guild()
        })?;

    let channels = fetch_guild_channels(state, guild_id).await?;
    let roles = fetch_guild_roles(state, guild_id).await?;

    let rows = sqlx::query_as::<_, MemberJoinRow>(
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
    let online_count = count_guild_online(state, guild_id).await.max(1);

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

async fn fetch_discoverable_rows(
    state: &AppState,
    search: Option<&str>,
) -> Result<(Vec<DiscoverableGuildRow>, HashMap<String, u64>), AppError> {
    let mut qb = QueryBuilder::<MySql>::new(
        r#"
        SELECT
            g.id, g.name, g.icon, g.banner, g.description,
            COUNT(DISTINCT gm.user_id) AS member_count
        FROM guilds g
        LEFT JOIN guild_members gm ON g.id = gm.guild_id
        WHERE g.discoverable = TRUE
        "#,
    );

    if let Some(q) = search.map(str::trim).filter(|s| !s.is_empty()) {
        let pattern = format!("%{}%", q.to_ascii_lowercase());
        qb.push(" AND (LOWER(g.name) LIKE ");
        qb.push_bind(pattern.clone());
        qb.push(" OR LOWER(COALESCE(g.description, '')) LIKE ");
        qb.push_bind(pattern);
        qb.push(")");
    }

    qb.push(" GROUP BY g.id ORDER BY member_count DESC, g.name ASC");

    let rows = qb
        .build_query_as::<DiscoverableGuildRow>()
        .fetch_all(&state.db)
        .await?;

    let online_ids = state.online_user_ids().await;
    let mut online_counts = HashMap::new();

    if !online_ids.is_empty() && !rows.is_empty() {
        let mut qb = QueryBuilder::<MySql>::new(
            "SELECT guild_id, COUNT(DISTINCT user_id) AS online_count FROM guild_members WHERE user_id IN (",
        );
        let mut sep = qb.separated(", ");
        for uid in &online_ids {
            sep.push_bind(uid);
        }
        qb.push(") GROUP BY guild_id");

        let counts = qb
            .build_query_as::<GuildOnlineCountRow>()
            .fetch_all(&state.db)
            .await?;

        for row in counts {
            online_counts.insert(row.guild_id, row.online_count.max(0) as u64);
        }
    }

    Ok((rows, online_counts))
}

pub async fn get_discovery_guilds(
    state: &AppState,
    params: DiscoverySearchQuery,
) -> Result<GuildDiscoveryResponse, AppError> {
    if let Some(cat) = params.category {
        if cat != DiscoveryCategory::Other as u8 {
            return Ok(GuildDiscoveryResponse {
                guilds: vec![],
                total: 0,
                category_counts: vec![],
            });
        }
    }

    let (rows, online_counts) = fetch_discoverable_rows(state, params.query.as_deref()).await?;

    let total = rows.len() as u64;
    let offset = params.offset.unwrap_or(0) as usize;
    let limit = params.limit.unwrap_or(24).clamp(1, 48) as usize;

    let guilds: Vec<DiscoverableGuild> = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|row| {
            let online_count = online_counts.get(&row.id).copied().unwrap_or(0);
            DiscoverableGuild {
                id: row.id,
                name: row.name,
                icon: row.icon,
                banner: row.banner,
                description: row.description,
                category_type: DiscoveryCategory::Other,
                primary_language: "en-US".to_string(),
                custom_tags: vec![],
                member_count: row.member_count.max(0) as u64,
                online_count,
                features: vec!["DISCOVERABLE".to_string()],
                badge_ids: vec![],
                verification_level: VerificationLevel::Low,
            }
        })
        .collect();

    let category_counts = if total > 0 {
        vec![CategoryCount {
            category_type: DiscoveryCategory::Other,
            count: total,
        }]
    } else {
        vec![]
    };

    Ok(GuildDiscoveryResponse {
        total,
        guilds,
        category_counts,
    })
}

pub async fn get_discoverable_guilds(
    state: &AppState,
) -> Result<DiscoverableGuildsResponse, AppError> {
    let (rows, online_counts) = fetch_discoverable_rows(state, None).await?;

    let guilds: Vec<RestDiscoverableGuild> = rows
        .into_iter()
        .map(|row| {
            let presence_count = online_counts.get(&row.id).copied().unwrap_or(0);
            RestDiscoverableGuild {
                id: row.id,
                name: row.name,
                description: row.description,
                icon: row.icon,
                banner: row.banner,
                splash: None,
                discovery_splash: None,
                features: vec!["DISCOVERABLE".to_string()],
                approximate_presence_count: presence_count,
                approximate_member_count: row.member_count.max(0) as u64,
                premium_subscription_count: 0,
            }
        })
        .collect();

    let total = guilds.len() as u64;

    Ok(DiscoverableGuildsResponse {
        limit: total,
        offset: 0,
        total,
        guilds,
    })
}