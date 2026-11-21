use std::collections::HashMap;

use chrono::Utc;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::config::GuildCreationPolicy;
use crate::constants::flags::{PermissionFlags, UserFlags};
use crate::constants::gateway::events;
use crate::constants::limits::{
    GUILD_LIST_PERMISSION_LIMIT, MAX_GUILD_NAME_LENGTH, MAX_GUILDS_PAGE_LIMIT, MAX_GUILDS_PER_USER,
};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, ChannelType, GuildChannel, PermissionOverwrite,
};
use crate::models::gateway::presence::{build_presence_value, serialize_presence_user};
use crate::models::guild::{BaseGuild, GatewayGuild, Guild};
use crate::models::rest::guilds::ListGuildsQuery;
use crate::models::guild_member::{
    BaseGuildMember, GatewayGuildMember, GuildMemberFields, GuildMemberResponse,
    GuildMemberUserRef, GuildMemberUserRow,
};
use crate::models::permission::OverwriteType;
use crate::models::role::{Role, RoleRow};
use crate::services::permission_service::RolePerm;
use crate::services::{channel_service, member_service, permission_service, role_service};
use crate::utils::snowflake::DISCORD_SNOWFLAKE;

#[derive(FromRow)]
struct GuildRoleBatchRow {
    guild_id: String,
    id: String,
    position: u32,
    name: String,
    color: u32,
    unicode_emoji: Option<String>,
    hoist: bool,
    mentionable: bool,
    permissions: u64,
}

#[derive(FromRow)]
struct SelfMemberBatchRow {
    guild_id: String,
    joined_at: u64,
    nick: Option<String>,
}

#[derive(FromRow)]
struct GuildMemberBatchRow {
    guild_id: String,
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
struct GuildMemberBatchRoleRow {
    guild_id: String,
    user_id: String,
    role_id: String,
}

struct GuildsBaseData {
    guilds: Vec<BaseGuild>,
    channels_by_guild: HashMap<String, Vec<ChannelResponse>>,
    roles_by_guild: HashMap<String, Vec<Role>>,
    role_perms_by_guild: HashMap<String, Vec<RolePerm>>,
    online_counts: HashMap<String, u64>,
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

    let mut tx = state.db.begin().await?;

    let (owner_flags,): (u32,) =
        sqlx::query_as("SELECT public_flags FROM users WHERE id = ? FOR UPDATE")
            .bind(owner_id)
            .fetch_optional(&mut *tx)
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
            .fetch_one(&mut *tx)
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
    state.dispatch_user(events::GUILD_CREATE, &gateway_guild, owner_id).await;

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
    crate::utils::snowflake::validate_snowflake_opt("before", query.before.as_deref())?;
    crate::utils::snowflake::validate_snowflake_opt("after", query.after.as_deref())?;

    let limit = query
        .limit
        .unwrap_or(MAX_GUILDS_PAGE_LIMIT)
        .clamp(1, MAX_GUILDS_PAGE_LIMIT);
    let include_permissions = limit <= GUILD_LIST_PERMISSION_LIMIT;
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

async fn load_guilds_base_data(
    state: &AppState,
    guild_ids: &[String],
) -> Result<GuildsBaseData, AppError> {
    if guild_ids.is_empty() {
        return Ok(GuildsBaseData {
            guilds: vec![],
            channels_by_guild: HashMap::new(),
            roles_by_guild: HashMap::new(),
            role_perms_by_guild: HashMap::new(),
            online_counts: HashMap::new(),
        });
    }

    let mut gqb = QueryBuilder::<MySql>::new("SELECT * FROM guilds WHERE id IN (");
    let mut sep = gqb.separated(", ");
    for gid in guild_ids {
        sep.push_bind(gid);
    }
    gqb.push(") ORDER BY CAST(id AS UNSIGNED) ASC");
    let guilds = gqb.build_query_as::<BaseGuild>().fetch_all(&state.db).await?;

    if guilds.is_empty() {
        return Ok(GuildsBaseData {
            guilds: vec![],
            channels_by_guild: HashMap::new(),
            roles_by_guild: HashMap::new(),
            role_perms_by_guild: HashMap::new(),
            online_counts: HashMap::new(),
        });
    }

    let mut cqb = QueryBuilder::<MySql>::new(
        "SELECT id, type, position, guild_id, parent_id, name, last_message_id FROM channels WHERE guild_id IN (",
    );
    let mut sep = cqb.separated(", ");
    for g in &guilds {
        sep.push_bind(&g.id);
    }
    cqb.push(") ORDER BY position ASC");
    let channel_rows = cqb.build_query_as::<GuildChannel>().fetch_all(&state.db).await?;

    let mut overwrites_by_channel: HashMap<String, Vec<PermissionOverwrite>> = HashMap::new();
    if !channel_rows.is_empty() {
        let mut oqb = QueryBuilder::<MySql>::new(
            "SELECT o.channel_id, o.type, o.target_id, o.allow, o.deny FROM channel_permission_overwrites o INNER JOIN channels c ON c.id = o.channel_id WHERE c.guild_id IN (",
        );
        let mut sep = oqb.separated(", ");
        for g in &guilds {
            sep.push_bind(&g.id);
        }
        oqb.push(")");
        let overwrite_rows = oqb
            .build_query_as::<ChannelOverwriteRow>()
            .fetch_all(&state.db)
            .await?;

        for ow in overwrite_rows {
            overwrites_by_channel
                .entry(ow.channel_id.clone())
                .or_default()
                .push(PermissionOverwrite::from(ow));
        }
    }

    let mut channels_by_guild: HashMap<String, Vec<ChannelResponse>> = HashMap::new();
    for ch in channel_rows {
        let gid = ch.guild_id.clone();
        let ows = overwrites_by_channel.remove(&ch.id).unwrap_or_default();
        channels_by_guild
            .entry(gid)
            .or_default()
            .push(ChannelResponse::from_parts(ch, ows));
    }

    let mut rqb = QueryBuilder::<MySql>::new(
        "SELECT guild_id, id, position, name, color, unicode_emoji, hoist, mentionable, permissions FROM roles WHERE guild_id IN (",
    );
    let mut sep = rqb.separated(", ");
    for g in &guilds {
        sep.push_bind(&g.id);
    }
    rqb.push(") ORDER BY position DESC");
    let role_rows = rqb
        .build_query_as::<GuildRoleBatchRow>()
        .fetch_all(&state.db)
        .await?;

    let mut roles_by_guild: HashMap<String, Vec<Role>> = HashMap::new();
    let mut role_perms_by_guild: HashMap<String, Vec<RolePerm>> = HashMap::new();
    for r in role_rows {
        role_perms_by_guild
            .entry(r.guild_id.clone())
            .or_default()
            .push(RolePerm {
                id: r.id.clone(),
                permissions: r.permissions,
            });
        roles_by_guild
            .entry(r.guild_id)
            .or_default()
            .push(Role::from(RoleRow {
                id: r.id,
                position: r.position,
                name: r.name,
                color: r.color,
                unicode_emoji: r.unicode_emoji,
                hoist: r.hoist,
                mentionable: r.mentionable,
                permissions: r.permissions,
            }));
    }

    let online_counts = state.hub.count_guilds_online(guild_ids);

    Ok(GuildsBaseData {
        guilds,
        channels_by_guild,
        roles_by_guild,
        role_perms_by_guild,
        online_counts,
    })
}

pub async fn build_gateway_guilds_batch(
    state: &AppState,
    guild_ids: &[String],
    user_id: &str,
) -> Result<Vec<GatewayGuild>, AppError> {
    let mut base = load_guilds_base_data(state, guild_ids).await?;
    if base.guilds.is_empty() {
        return Ok(vec![]);
    }

    let mut mqb = QueryBuilder::<MySql>::new(
        "SELECT guild_id, joined_at, nick FROM guild_members WHERE user_id = ",
    );
    mqb.push_bind(user_id);
    mqb.push(" AND guild_id IN (");
    let mut sep = mqb.separated(", ");
    for g in &base.guilds {
        sep.push_bind(&g.id);
    }
    mqb.push(")");
    let self_rows = mqb
        .build_query_as::<SelfMemberBatchRow>()
        .fetch_all(&state.db)
        .await?;

    let mut self_members: HashMap<String, BaseGuildMember> = HashMap::with_capacity(self_rows.len());
    for r in self_rows {
        self_members.insert(
            r.guild_id,
            BaseGuildMember {
                joined_at: r.joined_at,
                nick: r.nick,
            },
        );
    }

    let mut mrqb = QueryBuilder::<MySql>::new(
        "SELECT guild_id, role_id FROM guild_member_roles WHERE user_id = ",
    );
    mrqb.push_bind(user_id);
    mrqb.push(" AND role_id != guild_id AND guild_id IN (");
    let mut sep = mrqb.separated(", ");
    for g in &base.guilds {
        sep.push_bind(&g.id);
    }
    mrqb.push(")");
    let role_rows: Vec<(String, String)> = mrqb.build_query_as().fetch_all(&state.db).await?;

    let mut self_roles: HashMap<String, Vec<String>> = HashMap::new();
    for (gid, rid) in role_rows {
        self_roles.entry(gid).or_default().push(rid);
    }

    let mut cqb = QueryBuilder::<MySql>::new(
        "SELECT guild_id, COUNT(*) AS cnt FROM guild_members WHERE guild_id IN (",
    );
    let mut sep = cqb.separated(", ");
    for g in &base.guilds {
        sep.push_bind(&g.id);
    }
    cqb.push(") GROUP BY guild_id");
    let count_rows: Vec<(String, i64)> = cqb.build_query_as().fetch_all(&state.db).await?;
    let member_counts: HashMap<String, i64> = count_rows.into_iter().collect();

    let mut out = Vec::with_capacity(base.guilds.len());
    for guild in base.guilds {
        let Some(member) = self_members.remove(&guild.id) else {
            continue;
        };
        let roles = base.roles_by_guild.remove(&guild.id).unwrap_or_default();
        let role_perms = base.role_perms_by_guild.remove(&guild.id).unwrap_or_default();
        let member_roles = self_roles.remove(&guild.id).unwrap_or_default();
        let all_channels = base.channels_by_guild.remove(&guild.id).unwrap_or_default();
        let channels = permission_service::filter_visible_channels_with_context(
            &guild.owner_id,
            &guild.id,
            user_id,
            &role_perms,
            &member_roles,
            all_channels,
        );
        let member_count = member_counts.get(&guild.id).copied().unwrap_or(1).max(1) as u64;
        let online_count = base.online_counts.get(&guild.id).copied().unwrap_or(0);

        out.push(GatewayGuild {
            id: guild.id.clone(),
            joined_at: member.joined_at,
            member_count,
            online_count,
            channels,
            roles,
            members: vec![GatewayGuildMember {
                user: GuildMemberUserRef {
                    id: user_id.to_string(),
                },
                member: GuildMemberFields::new(member.nick, member_roles, member.joined_at),
            }],
            emojis: vec![],
            stickers: vec![],
            presences: vec![],
            voice_states: vec![],
            threads: vec![],
            properties: Guild::from(guild),
        });
    }

    Ok(out)
}

pub async fn build_full_gateway_guilds_batch(
    state: &AppState,
    guild_ids: &[String],
    user_id: &str,
) -> Result<Vec<GatewayGuild<GuildMemberResponse>>, AppError> {
    let mut base = load_guilds_base_data(state, guild_ids).await?;
    if base.guilds.is_empty() {
        return Ok(vec![]);
    }

    let mut mqb = QueryBuilder::<MySql>::new(
        r#"
        SELECT
            gm.guild_id, gm.user_id, gm.joined_at, gm.nick,
            u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
        FROM guild_members gm
        INNER JOIN users u ON u.id = gm.user_id
        WHERE gm.guild_id IN (
        "#,
    );
    let mut sep = mqb.separated(", ");
    for g in &base.guilds {
        sep.push_bind(&g.id);
    }
    mqb.push(") ORDER BY gm.joined_at ASC");
    let member_rows = mqb
        .build_query_as::<GuildMemberBatchRow>()
        .fetch_all(&state.db)
        .await?;

    let mut mrqb = QueryBuilder::<MySql>::new(
        "SELECT guild_id, user_id, role_id FROM guild_member_roles WHERE role_id != guild_id AND guild_id IN (",
    );
    let mut sep = mrqb.separated(", ");
    for g in &base.guilds {
        sep.push_bind(&g.id);
    }
    mrqb.push(")");
    let role_rows = mrqb
        .build_query_as::<GuildMemberBatchRoleRow>()
        .fetch_all(&state.db)
        .await?;

    let mut roles_by_guild_user: HashMap<String, HashMap<String, Vec<String>>> = HashMap::new();
    for r in role_rows {
        roles_by_guild_user
            .entry(r.guild_id)
            .or_default()
            .entry(r.user_id)
            .or_default()
            .push(r.role_id);
    }

    let mut members_by_guild: HashMap<String, Vec<GuildMemberBatchRow>> = HashMap::new();
    for m in member_rows {
        members_by_guild.entry(m.guild_id.clone()).or_default().push(m);
    }

    let mut out = Vec::with_capacity(base.guilds.len());
    for guild in base.guilds {
        let raw_members = members_by_guild.remove(&guild.id).unwrap_or_default();
        let mut guild_member_roles = roles_by_guild_user.remove(&guild.id).unwrap_or_default();
        let viewer_roles = guild_member_roles.get(user_id).cloned().unwrap_or_default();

        let roles = base.roles_by_guild.remove(&guild.id).unwrap_or_default();
        let role_perms = base.role_perms_by_guild.remove(&guild.id).unwrap_or_default();
        let all_channels = base.channels_by_guild.remove(&guild.id).unwrap_or_default();
        let channels = permission_service::filter_visible_channels_with_context(
            &guild.owner_id,
            &guild.id,
            user_id,
            &role_perms,
            &viewer_roles,
            all_channels,
        );

        let joined_at = raw_members
            .iter()
            .find(|m| m.user_id == user_id)
            .map(|m| m.joined_at)
            .unwrap_or(guild.created_at);

        let members: Vec<GuildMemberResponse> = raw_members
            .into_iter()
            .map(|m| {
                let m_roles = guild_member_roles.remove(&m.user_id).unwrap_or_default();
                GuildMemberUserRow {
                    user_id: m.user_id,
                    joined_at: m.joined_at,
                    nick: m.nick,
                    username: m.username,
                    discriminator: m.discriminator,
                    global_name: m.global_name,
                    avatar: m.avatar,
                    bot: m.bot,
                    public_flags: m.public_flags,
                }
                .into_member_response(m_roles)
            })
            .collect();

        let member_count = members.len().max(1) as u64;
        let online_count = base.online_counts.get(&guild.id).copied().unwrap_or(0);
        let live_presences = state.hub.guild_live_presences(&guild.id);
        let presences: Vec<serde_json::Value> = if live_presences.is_empty() {
            vec![]
        } else {
            members
                .iter()
                .filter_map(|m| {
                    let (status, afk, mobile, custom_status) = live_presences.get(&m.user.id)?;
                    if *status == "offline" {
                        return None;
                    }
                    let user_val = serialize_presence_user(&m.user.id, Some(&m.user));
                    Some(build_presence_value(
                        &user_val,
                        status,
                        *mobile,
                        *afk,
                        custom_status.as_ref(),
                        None,
                    ))
                })
                .collect()
        };

        out.push(GatewayGuild {
            id: guild.id.clone(),
            joined_at,
            member_count,
            online_count,
            channels,
            roles,
            members,
            emojis: vec![],
            stickers: vec![],
            presences,
            voice_states: vec![],
            threads: vec![],
            properties: Guild::from(guild),
        });
    }

    Ok(out)
}

pub async fn build_full_gateway_guild(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<GatewayGuild<GuildMemberResponse>, AppError> {
    build_full_gateway_guilds_batch(state, &[guild_id.to_string()], user_id)
        .await?
        .into_iter()
        .next()
        .ok_or_else(AppError::unknown_guild)
}