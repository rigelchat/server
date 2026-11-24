use std::collections::HashMap;

use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::error::AppError;
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, ChannelRow, PermissionOverwrite,
};
use crate::models::gateway::presence::{GatewayPresence, PresenceUser};
use crate::models::guild::{BaseGuild, GatewayGuild, Guild};
use crate::models::guild_member::{
    BaseGuildMember, GatewayGuildMember, GuildMemberFields, GuildMemberResponse,
    GuildMemberUserRef, GuildMemberUserRow,
};
use crate::models::role::{Role, RolePerm, RoleRow};
use crate::services::permission_service;

#[derive(FromRow)]
struct GuildRoleBatchRow {
    guild_id: String,
    id: String,
    position: u32,
    name: String,
    color: u32,
    hoist_position: Option<i32>,
    icon: Option<String>,
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
    let guilds = gqb
        .build_query_as::<BaseGuild>()
        .fetch_all(&state.db)
        .await?;

    if guilds.is_empty() {
        return Ok(GuildsBaseData {
            guilds: vec![],
            channels_by_guild: HashMap::new(),
            roles_by_guild: HashMap::new(),
            role_perms_by_guild: HashMap::new(),
            online_counts: HashMap::new(),
        });
    }

    let mut cqb = QueryBuilder::<MySql>::new(format!(
        "SELECT {} FROM channels WHERE guild_id IN (",
        ChannelRow::SELECT_COLS
    ));
    let mut sep = cqb.separated(", ");
    for g in &guilds {
        sep.push_bind(&g.id);
    }
    cqb.push(") ORDER BY position ASC");
    let channel_rows = cqb
        .build_query_as::<ChannelRow>()
        .fetch_all(&state.db)
        .await?;

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
            .push(ChannelResponse::from_row(ch, ows));
    }

    let mut rqb = QueryBuilder::<MySql>::new(
        "SELECT guild_id, id, position, name, color, hoist_position, icon, unicode_emoji, hoist, mentionable, permissions FROM roles WHERE guild_id IN (",
    );
    let mut sep = rqb.separated(", ");
    for g in &guilds {
        sep.push_bind(&g.id);
    }
    rqb.push(") ORDER BY position DESC, CAST(id AS UNSIGNED) DESC");
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
                hoist_position: r.hoist_position,
                icon: r.icon,
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

    let mut self_members: HashMap<String, BaseGuildMember> =
        HashMap::with_capacity(self_rows.len());
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
        let role_perms = base
            .role_perms_by_guild
            .remove(&guild.id)
            .unwrap_or_default();
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

async fn build_full_gateway_guilds_batch(
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
        members_by_guild
            .entry(m.guild_id.clone())
            .or_default()
            .push(m);
    }

    let mut out = Vec::with_capacity(base.guilds.len());
    for guild in base.guilds {
        let raw_members = members_by_guild.remove(&guild.id).unwrap_or_default();
        let mut guild_member_roles = roles_by_guild_user.remove(&guild.id).unwrap_or_default();
        let viewer_roles = guild_member_roles.get(user_id).cloned().unwrap_or_default();

        let roles = base.roles_by_guild.remove(&guild.id).unwrap_or_default();
        let role_perms = base
            .role_perms_by_guild
            .remove(&guild.id)
            .unwrap_or_default();
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
        let presences: Vec<GatewayPresence> = if live_presences.is_empty() {
            vec![]
        } else {
            members
                .iter()
                .filter_map(|m| {
                    let (status, afk, mobile, custom_status) = live_presences.get(&m.user.id)?;
                    if *status == "offline" {
                        return None;
                    }
                    Some(GatewayPresence {
                        user: PresenceUser::Partial(m.user.clone()),
                        status: (*status).to_string(),
                        mobile: *mobile,
                        afk: *afk,
                        custom_status: custom_status.clone(),
                        guild_id: None,
                    })
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
