use std::collections::{HashMap, HashSet};

use chrono::Utc;
use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::config::GuildCreationPolicy;
use crate::constants::flags::{PermissionFlags, UserFlags};
use crate::constants::gateway::events;
use crate::constants::guild::{
    DEFAULT_VOICE_BITRATE, DEFAULT_VOICE_CONNECTION_LIMIT, MAX_AFK_TIMEOUT,
    MAX_DEFAULT_MESSAGE_NOTIFICATIONS, MAX_EXPLICIT_CONTENT_FILTER,
    MAX_GUILD_DESCRIPTION_LENGTH, MAX_VERIFICATION_LEVEL, MAX_VOICE_BITRATE_STANDARD,
    MAX_VOICE_CONNECTION_LIMIT, MAX_VOICE_USER_LIMIT, MIN_AFK_TIMEOUT, MIN_VOICE_BITRATE,
    MIN_VOICE_CONNECTION_LIMIT, SYSTEM_CHANNEL_FLAGS_MASK, VerificationLevel,
};
use crate::constants::limits::{
    GUILD_LIST_PERMISSION_LIMIT, MAX_ASSET_HASH_LENGTH, MAX_CHANNEL_NAME_LENGTH,
    MAX_CHANNEL_RATE_LIMIT_PER_USER, MAX_CHANNEL_TOPIC_LENGTH, MAX_CHANNELS_PER_GUILD,
    MAX_COLOR_VALUE, MAX_GUILD_NAME_LENGTH, MAX_GUILD_ROLES, MAX_GUILDS_PAGE_LIMIT,
    MAX_GUILDS_PER_USER, MAX_ROLE_NAME_LENGTH, MAX_ROLE_UNICODE_EMOJI_LENGTH,
};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, ChannelRow, ChannelType, PermissionOverwrite,
};
use crate::models::gateway::events::GuildDeleteEvent;
use crate::models::gateway::presence::{build_presence_value, serialize_presence_user};
use crate::models::guild::{BaseGuild, GatewayGuild, Guild};
use crate::models::guild_member::{
    BaseGuildMember, GatewayGuildMember, GuildMemberFields, GuildMemberResponse,
    GuildMemberUserRef, GuildMemberUserRow,
};
use crate::models::permission::OverwriteType;
use crate::models::rest::guilds::{
    CreateGuildRequest, GuildCreationTemplate, ListGuildsQuery, UpdateGuildRequest,
};
use crate::models::role::{Role, RolePerm, RoleRow};
use crate::services::{channel_service, member_service, permission_service, role_service};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake};

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

struct PreparedTemplateRole {
    id: String,
    position: u32,
    name: String,
    color: u32,
    unicode_emoji: Option<String>,
    hoist: bool,
    mentionable: bool,
    permissions: u64,
}

struct PreparedTemplateOverwrite {
    target_id: String,
    allow: u64,
    deny: u64,
}

struct PreparedTemplateChannel {
    id: String,
    kind: u32,
    position: u32,
    parent_id: Option<String>,
    name: String,
    topic: Option<String>,
    nsfw: bool,
    rate_limit_per_user: u32,
    bitrate: Option<u32>,
    user_limit: Option<u32>,
    voice_connection_limit: Option<u32>,
    overwrites: Vec<PreparedTemplateOverwrite>,
}

struct PreparedTemplate {
    verification_level: u8,
    default_message_notifications: u8,
    explicit_content_filter: u8,
    afk_timeout: u32,
    system_channel_flags: u32,
    system_channel_id: String,
    everyone_permissions: u64,
    everyone_color: u32,
    roles: Vec<PreparedTemplateRole>,
    channels: Vec<PreparedTemplateChannel>,
}

fn prepare_template(
    guild_id: &str,
    tpl: GuildCreationTemplate,
) -> Result<PreparedTemplate, AppError> {
    let raw_roles = tpl.roles.unwrap_or_default();
    let raw_channels = tpl.channels.unwrap_or_default();

    if raw_roles.len() > MAX_GUILD_ROLES as usize {
        return Err(AppError::guild_template_invalid(
            "Guild template exceeds maximum number of roles.",
        ));
    }
    if raw_channels.len() > MAX_CHANNELS_PER_GUILD as usize {
        return Err(AppError::guild_template_invalid(
            "Guild template exceeds maximum number of channels.",
        ));
    }

    let mut seen_role_ids = HashSet::with_capacity(raw_roles.len());
    for role in &raw_roles {
        let tid = role.id.trim();
        if tid.is_empty() || !seen_role_ids.insert(tid.to_string()) {
            return Err(AppError::guild_template_invalid(
                "Template role IDs must be non-empty and unique.",
            ));
        }
    }

    let mut seen_channel_ids = HashSet::with_capacity(raw_channels.len());
    for ch in &raw_channels {
        let tid = ch.id.trim();
        if tid.is_empty() || !seen_channel_ids.insert(tid.to_string()) {
            return Err(AppError::guild_template_invalid(
                "Template channel IDs must be non-empty and unique.",
            ));
        }
    }

    let everyone_idx = raw_roles
        .iter()
        .position(|r| r.name.as_deref().map(str::trim) == Some("@everyone"))
        .or_else(|| raw_roles.iter().position(|r| r.id.trim() == "0"));

    let mut everyone_permissions = PermissionFlags::DEFAULT_EVERYONE.bits();
    let mut everyone_color = 0u32;
    let mut role_id_map: HashMap<String, String> = HashMap::with_capacity(raw_roles.len() + 1);
    role_id_map.insert("0".to_string(), guild_id.to_string());

    if let Some(idx) = everyone_idx {
        let ev = &raw_roles[idx];
        if let Some(bits) = ev.permissions_new.or(ev.permissions) {
            everyone_permissions = PermissionFlags::from_bits_truncate(bits).bits();
        }
        everyone_color = ev.color.unwrap_or(0).min(MAX_COLOR_VALUE);
        role_id_map.insert(ev.id.trim().to_string(), guild_id.to_string());
    }

    let mut roles = Vec::with_capacity(raw_roles.len());
    let mut next_role_pos = 1u32;
    for (idx, r) in raw_roles.into_iter().enumerate() {
        if Some(idx) == everyone_idx {
            continue;
        }
        let new_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        role_id_map.insert(r.id.trim().to_string(), new_id.clone());

        let name: String = r
            .name
            .as_deref()
            .map(str::trim)
            .unwrap_or("")
            .chars()
            .take(MAX_ROLE_NAME_LENGTH)
            .collect();
        let permissions = r
            .permissions_new
            .or(r.permissions)
            .map(|bits| PermissionFlags::from_bits_truncate(bits).bits())
            .unwrap_or(0);

        roles.push(PreparedTemplateRole {
            id: new_id,
            position: next_role_pos,
            name,
            color: r.color.unwrap_or(0).min(MAX_COLOR_VALUE),
            unicode_emoji: sanitize_template_unicode_emoji(r.unicode_emoji.as_deref()),
            hoist: r.hoist.unwrap_or(false),
            mentionable: r.mentionable.unwrap_or(false),
            permissions,
        });
        next_role_pos += 1;
    }

    let mut category_id_map: HashMap<String, String> = HashMap::new();
    let mut kept_channels = Vec::with_capacity(raw_channels.len());
    for ch in raw_channels {
        let mapped_kind = match ch.kind {
            0 => ChannelType::GuildText as u32,
            2 | 13 => ChannelType::GuildVoice as u32,
            4 => ChannelType::GuildCategory as u32,
            5 => ChannelType::GuildAnnouncement as u32,
            _ => continue,
        };
        let new_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        if mapped_kind == ChannelType::GuildCategory as u32 {
            category_id_map.insert(ch.id.trim().to_string(), new_id.clone());
        }
        kept_channels.push((new_id, mapped_kind, ch));
    }

    let mut categories = Vec::new();
    let mut children = Vec::new();
    let mut template_ch_id_map: HashMap<String, (String, u32)> = HashMap::new();
    let mut first_text_channel: Option<(u32, usize, String)> = None;

    for (order_idx, (new_id, mapped_kind, ch)) in kept_channels.into_iter().enumerate() {
        let tpl_id = ch.id.trim().to_string();
        let position = ch.position.unwrap_or(0).clamp(0, u32::MAX as i64) as u32;
        let parent_id = if mapped_kind == ChannelType::GuildCategory as u32 {
            None
        } else {
            ch.parent_id
                .as_deref()
                .map(str::trim)
                .and_then(|pid| category_id_map.get(pid).cloned())
        };

        let name = normalize_template_channel_name(ch.name.as_deref(), mapped_kind);
        let topic = ch
            .topic
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.chars().take(MAX_CHANNEL_TOPIC_LENGTH).collect());
        let is_voice = mapped_kind == ChannelType::GuildVoice as u32;
        let bitrate = is_voice.then(|| {
            ch.bitrate
                .unwrap_or(DEFAULT_VOICE_BITRATE)
                .clamp(MIN_VOICE_BITRATE, MAX_VOICE_BITRATE_STANDARD)
        });
        let user_limit = is_voice.then(|| ch.user_limit.unwrap_or(0).min(MAX_VOICE_USER_LIMIT));
        let voice_connection_limit = is_voice.then(|| {
            ch.voice_connection_limit
                .unwrap_or(DEFAULT_VOICE_CONNECTION_LIMIT)
                .clamp(MIN_VOICE_CONNECTION_LIMIT, MAX_VOICE_CONNECTION_LIMIT)
        });
        let rate_limit_per_user = ch
            .rate_limit_per_user
            .unwrap_or(0)
            .min(MAX_CHANNEL_RATE_LIMIT_PER_USER);

        let mut overwrites: Vec<PreparedTemplateOverwrite> = Vec::new();
        let mut ow_index: HashMap<String, usize> = HashMap::new();
        for ow in ch.permission_overwrites.unwrap_or_default() {
            if ow.kind != Some(OverwriteType::Role as u32) {
                continue;
            }
            let Some(target_id) = role_id_map.get(ow.id.trim()).cloned() else {
                continue;
            };
            let prepared_ow = PreparedTemplateOverwrite {
                target_id: target_id.clone(),
                allow: PermissionFlags::from_bits_truncate(ow.allow.unwrap_or(0)).bits(),
                deny: PermissionFlags::from_bits_truncate(ow.deny.unwrap_or(0)).bits(),
            };
            if let Some(&existing_idx) = ow_index.get(&target_id) {
                overwrites[existing_idx] = prepared_ow;
            } else {
                ow_index.insert(target_id, overwrites.len());
                overwrites.push(prepared_ow);
            }
        }

        if mapped_kind == ChannelType::GuildText as u32
            || mapped_kind == ChannelType::GuildAnnouncement as u32
        {
            template_ch_id_map.insert(tpl_id, (new_id.clone(), mapped_kind));
            let replace_first = match &first_text_channel {
                None => true,
                Some((best_pos, best_ord, _)) => (position, order_idx) < (*best_pos, *best_ord),
            };
            if replace_first {
                first_text_channel = Some((position, order_idx, new_id.clone()));
            }
        }

        let prepared_ch = PreparedTemplateChannel {
            id: new_id,
            kind: mapped_kind,
            position,
            parent_id,
            name,
            topic,
            nsfw: ch.nsfw.unwrap_or(false),
            rate_limit_per_user,
            bitrate,
            user_limit,
            voice_connection_limit,
            overwrites,
        };

        if mapped_kind == ChannelType::GuildCategory as u32 {
            categories.push(prepared_ch);
        } else {
            children.push(prepared_ch);
        }
    }

    let mut channels = categories;
    channels.extend(children);

    let requested_sys = tpl
        .system_channel_id
        .as_deref()
        .map(str::trim)
        .and_then(|tid| template_ch_id_map.get(tid))
        .map(|(new_id, _)| new_id.clone());

    let system_channel_id = if let Some(id) = requested_sys {
        id
    } else if let Some((_, _, first_id)) = first_text_channel {
        first_id
    } else {
        if channels.len() >= MAX_CHANNELS_PER_GUILD as usize {
            return Err(AppError::guild_template_invalid(
                "Guild template exceeds maximum number of channels.",
            ));
        }
        let fallback_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        channels.push(PreparedTemplateChannel {
            id: fallback_id.clone(),
            kind: ChannelType::GuildText as u32,
            position: 0,
            parent_id: None,
            name: "general".to_string(),
            topic: None,
            nsfw: false,
            rate_limit_per_user: 0,
            bitrate: None,
            user_limit: None,
            voice_connection_limit: None,
            overwrites: Vec::new(),
        });
        fallback_id
    };

    Ok(PreparedTemplate {
        verification_level: tpl
            .verification_level
            .unwrap_or(0)
            .clamp(0, MAX_VERIFICATION_LEVEL as i64) as u8,
        default_message_notifications: tpl
            .default_message_notifications
            .unwrap_or(0)
            .clamp(0, MAX_DEFAULT_MESSAGE_NOTIFICATIONS as i64) as u8,
        explicit_content_filter: tpl
            .explicit_content_filter
            .unwrap_or(0)
            .clamp(0, MAX_EXPLICIT_CONTENT_FILTER as i64) as u8,
        afk_timeout: tpl
            .afk_timeout
            .unwrap_or(300)
            .clamp(MIN_AFK_TIMEOUT as i64, MAX_AFK_TIMEOUT as i64) as u32,
        system_channel_flags: tpl.system_channel_flags.unwrap_or(0) & SYSTEM_CHANNEL_FLAGS_MASK,
        system_channel_id,
        everyone_permissions,
        everyone_color,
        roles,
        channels,
    })
}

fn sanitize_template_unicode_emoji(raw: Option<&str>) -> Option<String> {
    let trimmed = raw?.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_ROLE_UNICODE_EMOJI_LENGTH
        || trimmed
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn normalize_template_channel_name(raw: Option<&str>, kind: u32) -> String {
    let input = raw.unwrap_or("");
    let mut collapsed = String::with_capacity(input.len());
    let mut prev_space = true;
    for c in input.chars() {
        if c.is_control() {
            continue;
        }
        if c.is_whitespace() {
            if !prev_space {
                collapsed.push(' ');
                prev_space = true;
            }
        } else {
            collapsed.push(c);
            prev_space = false;
        }
    }
    let trimmed = collapsed.trim();
    let normalized = if kind == ChannelType::GuildText as u32
        || kind == ChannelType::GuildAnnouncement as u32
    {
        let out: String = trimmed
            .to_lowercase()
            .chars()
            .filter_map(|c| {
                if c == ' ' {
                    Some('-')
                } else if !c.is_ascii_punctuation() || c == '-' || c == '.' || c == '_' {
                    Some(c)
                } else {
                    None
                }
            })
            .collect();
        if out.is_empty() {
            trimmed.to_string()
        } else {
            out
        }
    } else {
        trimmed.to_string()
    };

    if normalized.is_empty() {
        "channel".to_string()
    } else {
        normalized.chars().take(MAX_CHANNEL_NAME_LENGTH).collect()
    }
}

pub async fn create_guild(
    state: &AppState,
    payload: CreateGuildRequest,
    owner_id: &str,
) -> Result<Guild, AppError> {
    let name = payload.name.trim();
    if name.is_empty() || name.chars().count() > MAX_GUILD_NAME_LENGTH {
        return Err(AppError::validation(
            "name",
            ValidationCode::StringLengthInvalid,
            "String length must be between 1 and 100 characters.",
        ));
    }

    let guild_id = DISCORD_SNOWFLAKE.generate(None).to_string();
    let prepared_template = payload
        .template
        .map(|tpl| prepare_template(&guild_id, tpl))
        .transpose()?;

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

    if let Some(tpl) = prepared_template {
        sqlx::query(
            r#"
            INSERT INTO guilds (
                id, created_at, owner_id, name, system_channel_id, system_channel_flags,
                afk_timeout, verification_level, default_message_notifications, explicit_content_filter
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(&guild_id)
        .bind(now)
        .bind(owner_id)
        .bind(name)
        .bind(&tpl.system_channel_id)
        .bind(tpl.system_channel_flags)
        .bind(tpl.afk_timeout)
        .bind(tpl.verification_level)
        .bind(tpl.default_message_notifications)
        .bind(tpl.explicit_content_filter)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO roles (id, guild_id, position, name, color, permissions) VALUES (?, ?, 0, '@everyone', ?, ?)",
        )
        .bind(&guild_id)
        .bind(&guild_id)
        .bind(tpl.everyone_color)
        .bind(tpl.everyone_permissions)
        .execute(&mut *tx)
        .await?;

        for role in tpl.roles {
            sqlx::query(
                r#"
                INSERT INTO roles (id, guild_id, position, name, color, unicode_emoji, hoist, mentionable, permissions)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&role.id)
            .bind(&guild_id)
            .bind(role.position)
            .bind(&role.name)
            .bind(role.color)
            .bind(&role.unicode_emoji)
            .bind(role.hoist)
            .bind(role.mentionable)
            .bind(role.permissions)
            .execute(&mut *tx)
            .await?;
        }

        for ch in tpl.channels {
            sqlx::query(
                r#"
                INSERT INTO channels (
                    id, type, position, guild_id, parent_id, name, topic, nsfw,
                    rate_limit_per_user, bitrate, user_limit, voice_connection_limit
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&ch.id)
            .bind(ch.kind)
            .bind(ch.position)
            .bind(&guild_id)
            .bind(&ch.parent_id)
            .bind(&ch.name)
            .bind(&ch.topic)
            .bind(ch.nsfw)
            .bind(ch.rate_limit_per_user)
            .bind(ch.bitrate)
            .bind(ch.user_limit)
            .bind(ch.voice_connection_limit)
            .execute(&mut *tx)
            .await?;

            for ow in ch.overwrites {
                sqlx::query(
                    "INSERT INTO channel_permission_overwrites (type, channel_id, target_id, allow, deny) VALUES (?, ?, ?, ?, ?)",
                )
                .bind(OverwriteType::Role as u32)
                .bind(&ch.id)
                .bind(&ow.target_id)
                .bind(ow.allow)
                .bind(ow.deny)
                .execute(&mut *tx)
                .await?;
            }
        }
    } else {
        let text_category_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        let voice_category_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        let system_channel_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        let afk_channel_id = DISCORD_SNOWFLAKE.generate(None).to_string();
        let default_permissions = PermissionFlags::DEFAULT_EVERYONE.bits();

        let channels = [
            (
                &text_category_id,
                ChannelType::GuildCategory,
                0u32,
                None,
                "Text channels",
            ),
            (
                &voice_category_id,
                ChannelType::GuildCategory,
                1u32,
                None,
                "Voice channels",
            ),
            (
                &system_channel_id,
                ChannelType::GuildText,
                0u32,
                Some(&text_category_id),
                "general",
            ),
            (
                &afk_channel_id,
                ChannelType::GuildVoice,
                0u32,
                Some(&voice_category_id),
                "General",
            ),
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
        .dispatch_user(events::GUILD_CREATE, &gateway_guild, owner_id)
        .await;

    Ok(props)
}

pub async fn get_guild(state: &AppState, guild_id: &str, user_id: &str) -> Result<Guild, AppError> {
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

pub async fn update_guild(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: UpdateGuildRequest,
) -> Result<Guild, AppError> {
    let perms = permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !perms.contains(PermissionFlags::MANAGE_GUILD) {
        return Err(AppError::missing_permissions());
    }

    let mut tx = state.db.begin().await?;

    let current = sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ? FOR UPDATE")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let mut query = QueryBuilder::<MySql>::new("UPDATE guilds SET ");
    let mut sep = query.separated(", ");
    let mut has_updates = false;

    if let Some(raw_name) = &payload.name {
        let trimmed = raw_name.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_GUILD_NAME_LENGTH {
            return Err(AppError::validation(
                "name",
                ValidationCode::StringLengthInvalid,
                "String length must be between 1 and 100 characters.",
            ));
        }
        sep.push("name = ");
        sep.push_bind_unseparated(trimmed.to_string());
        has_updates = true;
    }

    if let Some(icon) = &payload.icon {
        let value = validate_guild_asset_opt("icon", icon.as_deref())?;
        sep.push("icon = ");
        sep.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(banner) = &payload.banner {
        let value = validate_guild_asset_opt("banner", banner.as_deref())?;
        sep.push("banner = ");
        sep.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(description) = &payload.description {
        let value = description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let Some(text) = value
            && text.chars().count() > MAX_GUILD_DESCRIPTION_LENGTH
        {
            return Err(AppError::validation(
                "description",
                ValidationCode::ContentExceedsMaxLength,
                "Description must not exceed 300 characters.",
            ));
        }
        sep.push("description = ");
        sep.push_bind_unseparated(value.map(str::to_string));
        has_updates = true;
    }

    if let Some(system_channel_id) = &payload.system_channel_id {
        let value = match system_channel_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(cid) => {
                validate_snowflake("system_channel_id", cid)?;
                let ch_type: Option<u32> =
                    sqlx::query_scalar("SELECT type FROM channels WHERE id = ? AND guild_id = ?")
                        .bind(cid)
                        .bind(guild_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                match ch_type {
                    None => {
                        return Err(AppError::validation(
                            "system_channel_id",
                            ValidationCode::SystemChannelMustBeInGuild,
                            "System channel must belong to this community.",
                        ));
                    }
                    Some(t)
                        if t != ChannelType::GuildText as u32
                            && t != ChannelType::GuildAnnouncement as u32 =>
                    {
                        return Err(AppError::validation(
                            "system_channel_id",
                            ValidationCode::SystemChannelMustBeText,
                            "System channel must be a text or announcement channel.",
                        ));
                    }
                    Some(_) => Some(cid.to_string()),
                }
            }
            None => None,
        };
        sep.push("system_channel_id = ");
        sep.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(flags) = payload.system_channel_flags {
        sep.push("system_channel_flags = ");
        sep.push_bind_unseparated(flags & SYSTEM_CHANNEL_FLAGS_MASK);
        has_updates = true;
    }

    if let Some(rules_channel_id) = &payload.rules_channel_id {
        let value = match rules_channel_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(cid) => {
                validate_snowflake("rules_channel_id", cid)?;
                let ch_type: Option<u32> =
                    sqlx::query_scalar("SELECT type FROM channels WHERE id = ? AND guild_id = ?")
                        .bind(cid)
                        .bind(guild_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                match ch_type {
                    None => {
                        return Err(AppError::validation(
                            "rules_channel_id",
                            ValidationCode::InvalidFormat,
                            "Rules channel must belong to this community.",
                        ));
                    }
                    Some(t)
                        if t != ChannelType::GuildText as u32
                            && t != ChannelType::GuildAnnouncement as u32 =>
                    {
                        return Err(AppError::validation(
                            "rules_channel_id",
                            ValidationCode::InvalidFormat,
                            "Rules channel must be a text or announcement channel.",
                        ));
                    }
                    Some(_) => Some(cid.to_string()),
                }
            }
            None => None,
        };
        sep.push("rules_channel_id = ");
        sep.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(afk_channel_id) = &payload.afk_channel_id {
        let value = match afk_channel_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(cid) => {
                validate_snowflake("afk_channel_id", cid)?;
                let ch_type: Option<u32> =
                    sqlx::query_scalar("SELECT type FROM channels WHERE id = ? AND guild_id = ?")
                        .bind(cid)
                        .bind(guild_id)
                        .fetch_optional(&mut *tx)
                        .await?;
                match ch_type {
                    None => {
                        return Err(AppError::validation(
                            "afk_channel_id",
                            ValidationCode::AfkChannelMustBeInGuild,
                            "AFK channel must belong to this community.",
                        ));
                    }
                    Some(t) if t != ChannelType::GuildVoice as u32 => {
                        return Err(AppError::validation(
                            "afk_channel_id",
                            ValidationCode::AfkChannelMustBeVoice,
                            "AFK channel must be a voice channel.",
                        ));
                    }
                    Some(_) => Some(cid.to_string()),
                }
            }
            None => None,
        };
        sep.push("afk_channel_id = ");
        sep.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(timeout) = payload.afk_timeout {
        if !(MIN_AFK_TIMEOUT..=MAX_AFK_TIMEOUT).contains(&timeout) {
            return Err(AppError::validation(
                "afk_timeout",
                ValidationCode::ValueMustBeIntegerInRange,
                "AFK timeout must be between 60 and 3600 seconds.",
            ));
        }
        sep.push("afk_timeout = ");
        sep.push_bind_unseparated(timeout);
        has_updates = true;
    }

    if let Some(notif) = payload.default_message_notifications {
        if notif > MAX_DEFAULT_MESSAGE_NOTIFICATIONS {
            return Err(AppError::validation(
                "default_message_notifications",
                ValidationCode::ValueMustBeIntegerInRange,
                "Invalid default message notifications value.",
            ));
        }
        sep.push("default_message_notifications = ");
        sep.push_bind_unseparated(notif);
        has_updates = true;
    }

    if let Some(level) = payload.verification_level {
        if level > MAX_VERIFICATION_LEVEL {
            return Err(AppError::validation(
                "verification_level",
                ValidationCode::ValueMustBeIntegerInRange,
                "Verification level must be between 0 and 3.",
            ));
        }
        if current.discoverable && level < VerificationLevel::Low as u8 {
            return Err(AppError::validation(
                "verification_level",
                ValidationCode::DiscoverableGuildVerificationLevelTooLow,
                "Discoverable communities must have at least low verification level.",
            ));
        }
        sep.push("verification_level = ");
        sep.push_bind_unseparated(level);
        has_updates = true;
    }

    if let Some(filter) = payload.explicit_content_filter {
        if filter > MAX_EXPLICIT_CONTENT_FILTER {
            return Err(AppError::validation(
                "explicit_content_filter",
                ValidationCode::ValueMustBeIntegerInRange,
                "Explicit content filter must be between 0 and 2.",
            ));
        }
        sep.push("explicit_content_filter = ");
        sep.push_bind_unseparated(filter);
        has_updates = true;
    }

    let resolved_nsfw = if let Some(nsfw) = payload.nsfw {
        Some(nsfw)
    } else if let Some(nsfw_level) = payload.nsfw_level {
        match nsfw_level {
            0 | 2 => Some(false),
            1 | 3 => Some(true),
            _ => {
                return Err(AppError::validation(
                    "nsfw_level",
                    ValidationCode::ValueMustBeIntegerInRange,
                    "Invalid NSFW level value.",
                ));
            }
        }
    } else {
        None
    };
    if let Some(nsfw) = resolved_nsfw {
        sep.push("nsfw = ");
        sep.push_bind_unseparated(nsfw);
        has_updates = true;
    }

    if !has_updates {
        tx.commit().await?;
        return Ok(Guild::from(current));
    }

    query.push(" WHERE id = ");
    query.push_bind(guild_id);
    query.build().execute(&mut *tx).await?;

    let updated_base = sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_one(&mut *tx)
        .await?;

    tx.commit().await?;

    let updated = Guild::from(updated_base);
    state
        .dispatch_guild(events::GUILD_UPDATE, &updated, guild_id)
        .await;

    Ok(updated)
}

pub async fn delete_guild(state: &AppState, guild_id: &str, user_id: &str) -> Result<(), AppError> {
    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    if owner_id != user_id {
        return Err(AppError::missing_permissions());
    }

    let res = sqlx::query("DELETE FROM guilds WHERE id = ?")
        .bind(guild_id)
        .execute(&state.db)
        .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::unknown_guild());
    }

    state
        .dispatch_guild(
            events::GUILD_DELETE,
            &GuildDeleteEvent { id: guild_id },
            guild_id,
        )
        .await;
    state.remove_guild(guild_id).await;

    Ok(())
}

fn validate_guild_asset_opt(field: &str, value: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.len() > MAX_ASSET_HASH_LENGTH
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AppError::validation(
            field,
            ValidationCode::InvalidFormat,
            format!("Invalid {field} identifier."),
        ));
    }
    Ok(Some(trimmed.to_string()))
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

    let rows = qb
        .build_query_as::<BaseGuild>()
        .fetch_all(&state.db)
        .await?;
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

    let online_counts = if with_counts {
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        state.hub.count_guilds_online(&ids)
    } else {
        HashMap::new()
    };

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
            guild.approximate_presence_count = Some(online_counts.get(&gid).copied().unwrap_or(0));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_template_maps_roles_categories_overwrites_and_fallbacks() {
        let json = r#"{
            "name": "Ignored",
            "verification_level": 9,
            "afk_timeout": 10,
            "system_channel_id": "99",
            "roles": [
                { "id": 0, "name": "@everyone", "permissions": "1024", "permissions_new": "2048" },
                { "id": 10, "name": "Mod", "color": 255, "hoist": true, "unicode_emoji": "🛡️" },
                { "id": "11", "name": "BadEmoji", "unicode_emoji": "not_emoji" }
            ],
            "channels": [
                { "id": 20, "type": 0, "name": "Welcome Chat!", "position": 0, "parent_id": 30,
                  "permission_overwrites": [
                    { "id": 0, "type": "role", "allow": "0", "deny": "1024" },
                    { "id": 10, "type": 0, "allow": "1024", "deny": "0" },
                    { "id": 999, "type": 0, "allow": "1024", "deny": "0" },
                    { "id": 10, "type": 1, "allow": "8", "deny": "0" }
                  ]
                },
                { "id": 30, "type": 4, "name": "Info", "position": 0 },
                { "id": 40, "type": 13, "name": "Stage", "position": 1, "bitrate": 384000 },
                { "id": 50, "type": 15, "name": "ForumSkipped", "position": 2 }
            ]
        }"#;

        let req: CreateGuildRequest = serde_json::from_str(&format!(r#"{{"name":"My Guild","template":{json}}}"#)).unwrap();
        let prepared = prepare_template("1000", req.template.unwrap()).unwrap();

        assert_eq!(prepared.verification_level, 3);
        assert_eq!(prepared.afk_timeout, 60);
        assert_eq!(prepared.everyone_permissions, 2048);
        assert_eq!(prepared.roles.len(), 2);
        assert_eq!(prepared.roles[0].name, "Mod");
        assert_eq!(prepared.roles[0].position, 1);
        assert_eq!(prepared.roles[0].unicode_emoji.as_deref(), Some("🛡️"));
        assert_eq!(prepared.roles[1].unicode_emoji, None);

        // Categories must come before children (Info, Welcome Chat, Stage); type 15 skipped
        assert_eq!(prepared.channels.len(), 3);
        assert_eq!(prepared.channels[0].kind, ChannelType::GuildCategory as u32);
        assert_eq!(prepared.channels[1].name, "welcome-chat");
        assert_eq!(prepared.channels[1].parent_id.as_deref(), Some(prepared.channels[0].id.as_str()));
        assert_eq!(prepared.channels[1].overwrites.len(), 2);
        assert_eq!(prepared.channels[1].overwrites[0].target_id, "1000");
        assert_eq!(prepared.channels[2].kind, ChannelType::GuildVoice as u32);
        assert_eq!(prepared.channels[2].bitrate, Some(MAX_VOICE_BITRATE_STANDARD));
        assert_eq!(prepared.system_channel_id, prepared.channels[1].id);
    }
}
