use std::collections::HashMap;

use chrono::Utc;
use serde::Serialize;
use serde_json::json;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::flags::{PermissionFlags, UserFlags};
use crate::constants::gateway::events;
use crate::constants::guild::{DiscoveryCategory, VerificationLevel};
use crate::error::AppError;
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, ChannelType, GuildChannel, PermissionOverwrite,
};
use crate::models::guild::{BaseGuild, GatewayGuild, GatewayGuildProperties};
use crate::models::guild_member::{
    BaseGuildMember, GatewayGuildMember, GuildMemberResponse, GuildMemberRole, GuildMemberUserRef,
};
use crate::models::permission::OverwriteType;
use crate::models::rest::discoverable_guilds::{
    DiscoverableGuild as RestDiscoverableGuild, DiscoverableGuildsResponse,
};
use crate::models::rest::discovery::{DiscoverableGuild, GuildDiscoveryResponse};
use crate::models::role::Role;
use crate::models::user::PublicUser;
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
    banner: Option<String>,
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
        return Err(AppError::Forbidden(
            "You are not a member of this guild".to_string(),
        ));
    }
    Ok(())
}

pub async fn create_guild(
    state: &AppState,
    name: &str,
    owner_id: &str,
) -> Result<GatewayGuild, AppError> {
    let name = name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::BadRequest(
            "Guild name must be between 1 and 100 characters".to_string(),
        ));
    }

    let (owner_flags,): (u32,) = sqlx::query_as("SELECT public_flags FROM users WHERE id = ?")
        .bind(owner_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::Unauthorized("User not found".to_string()))?;

    if !UserFlags::from_bits_truncate(owner_flags).contains(UserFlags::STAFF) {
        return Err(AppError::Forbidden(
            "Only staff users can create guilds".to_string(),
        ));
    }

    let now = Utc::now().timestamp_millis() as u64;
    let guild_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let text_category_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let voice_category_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let system_channel_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let afk_channel_id = DISCORD_SNOWFLAKE.generate(None).to_string();

    let default_permissions = (PermissionFlags::VIEW_CHANNEL
        | PermissionFlags::CREATE_EXPRESSIONS
        | PermissionFlags::SEND_MESSAGES
        | PermissionFlags::EMBED_LINKS
        | PermissionFlags::ATTACH_FILES
        | PermissionFlags::ADD_REACTIONS
        | PermissionFlags::READ_MESSAGE_HISTORY
        | PermissionFlags::CONNECT
        | PermissionFlags::SPEAK)
        .bits();

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

    sqlx::query("INSERT INTO guild_member_roles (guild_id, user_id, role_id) VALUES (?, ?, ?)")
        .bind(&guild_id)
        .bind(owner_id)
        .bind(&guild_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    state.add_user_guild(owner_id, &guild_id).await;

    let gateway_guild = build_gateway_guild(state, &guild_id, owner_id).await?;
    state
        .dispatch_user(events::GUILD_CREATE, &gateway_guild, owner_id)
        .await;

    Ok(gateway_guild)
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
        return Err(AppError::NotFound("Guild not found".to_string()));
    }

    let banned: Option<(String,)> =
        sqlx::query_as("SELECT id FROM guild_bans WHERE guild_id = ? AND user_id = ? LIMIT 1")
            .bind(guild_id)
            .bind(user_id)
            .fetch_optional(&state.db)
            .await?;

    if banned.is_some() {
        return Err(AppError::Forbidden("You are banned from this guild".to_string()));
    }

    if is_member(state, guild_id, user_id).await? {
        return Ok(());
    }

    let now = Utc::now().timestamp_millis() as u64;
    let mut tx = state.db.begin().await?;

    sqlx::query("INSERT INTO guild_members (user_id, guild_id, joined_at) VALUES (?, ?, ?)")
        .bind(user_id)
        .bind(guild_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;

    sqlx::query("INSERT INTO guild_member_roles (guild_id, user_id, role_id) VALUES (?, ?, ?)")
        .bind(guild_id)
        .bind(user_id)
        .bind(guild_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;

    // Build and broadcast GUILD_MEMBER_ADD to existing members before adding the joiner to session.guilds.
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

    let gateway_guild = build_gateway_guild(state, guild_id, user_id).await?;
    state
        .dispatch_user(events::GUILD_CREATE, &gateway_guild, user_id)
        .await;

    Ok(())
}

pub async fn leave_guild(state: &AppState, user_id: &str, guild_id: &str) -> Result<(), AppError> {
    if !is_member(state, guild_id, user_id).await? {
        return Err(AppError::NotFound("You are not in this guild".to_string()));
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

    state
        .dispatch_user(
            events::GUILD_DELETE,
            &json!({ "id": guild_id, "unavailable": false }),
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

pub async fn fetch_guild_channels(
    state: &AppState,
    guild_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    let channels = sqlx::query_as::<_, GuildChannel>(
        "SELECT id, type, position, guild_id, parent_id, name FROM channels WHERE guild_id = ? ORDER BY position ASC",
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
            ChannelResponse {
                base,
                nsfw: false,
                nsfw_override: false,
                permission_overwrites,
            }
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
    let roles = sqlx::query_as::<_, Role>(
        "SELECT id, guild_id, position, name, color, unicode_emoji, hoist, mentionable, permissions FROM roles WHERE guild_id = ? ORDER BY position ASC",
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    Ok(roles)
}

pub async fn get_guild_members(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<Vec<GuildMemberResponse>, AppError> {
    require_member(state, guild_id, user_id).await?;

    let rows = sqlx::query_as::<_, MemberJoinRow>(
        r#"
        SELECT
            gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.banner, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = ?
        ORDER BY gm.joined_at ASC
        LIMIT 100
        "#,
    )
    .bind(guild_id)
    .fetch_all(&state.db)
    .await?;

    let role_rows = sqlx::query_as::<_, GuildMemberRole>(
        "SELECT guild_id, user_id, role_id FROM guild_member_roles WHERE guild_id = ?",
    )
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
                user: PublicUser {
                    id: m.user_id,
                    username: m.username,
                    discriminator: m.discriminator,
                    global_name: m.global_name,
                    avatar: m.avatar,
                    banner: m.banner,
                    bot: m.bot,
                    flags: m.public_flags,
                },
                roles,
                nick: m.nick,
                avatar: None,
                premium_since: None,
                joined_at: m.joined_at,
                deaf: false,
                mute: false,
                flags: 0,
                pending: false,
            }
        })
        .collect())
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
            u.username, u.discriminator, u.global_name, u.avatar, u.banner, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id = ? AND gm.user_id = ?
        "#,
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;

    let roles: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    Ok(GuildMemberResponse {
        user: PublicUser {
            id: row.user_id,
            username: row.username,
            discriminator: row.discriminator,
            global_name: row.global_name,
            avatar: row.avatar,
            banner: row.banner,
            bot: row.bot,
            flags: row.public_flags,
        },
        roles,
        nick: row.nick,
        avatar: None,
        premium_since: None,
        joined_at: row.joined_at,
        deaf: false,
        mute: false,
        flags: 0,
        pending: false,
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
        .ok_or_else(|| AppError::NotFound("Guild not found".to_string()))?;

    let channels = fetch_guild_channels(state, guild_id).await?;
    let roles = fetch_guild_roles(state, guild_id).await?;

    let member = sqlx::query_as::<_, BaseGuildMember>(
        "SELECT joined_at, guild_id, user_id, nick FROM guild_members WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;

    let member_roles: Vec<String> = sqlx::query_scalar(
        "SELECT role_id FROM guild_member_roles WHERE guild_id = ? AND user_id = ?",
    )
    .bind(guild_id)
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    let member_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM guild_members WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_one(&state.db)
            .await?;

    let online_count = {
        let sessions = state.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.guilds.contains(guild_id))
            .filter_map(|s| s.user_id.as_ref())
            .collect::<std::collections::HashSet<_>>()
            .len()
            .max(1) as u64
    };

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
        properties: GatewayGuildProperties::from(guild),
    })
}

async fn fetch_discoverable_rows(
    state: &AppState,
) -> Result<(Vec<DiscoverableGuildRow>, HashMap<String, u64>), AppError> {
    let rows = sqlx::query_as::<_, DiscoverableGuildRow>(
        r#"
        SELECT
            g.id, g.name, g.icon, g.banner, g.description,
            COUNT(DISTINCT gm.user_id) AS member_count
        FROM guilds g
        LEFT JOIN guild_members gm ON g.id = gm.guild_id
        WHERE g.discoverable = TRUE
        GROUP BY g.id
        ORDER BY g.name ASC
        "#,
    )
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

pub async fn get_discovery_guilds(state: &AppState) -> Result<GuildDiscoveryResponse, AppError> {
    let (rows, online_counts) = fetch_discoverable_rows(state).await?;

    let guilds: Vec<DiscoverableGuild> = rows
        .into_iter()
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
                features: vec![],
                verification_level: VerificationLevel::None,
            }
        })
        .collect();

    let total = guilds.len() as u64;

    Ok(GuildDiscoveryResponse {
        total,
        guilds,
        category_counts: vec![],
    })
}

pub async fn get_discoverable_guilds(
    state: &AppState,
) -> Result<DiscoverableGuildsResponse, AppError> {
    let (rows, online_counts) = fetch_discoverable_rows(state).await?;

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
                features: vec![],
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