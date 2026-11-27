use std::collections::HashMap;

use chrono::Utc;
use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::config::GuildCreationPolicy;
use crate::constants::channel::{ChannelType, OverwriteType};
use crate::constants::flags::{PermissionFlags, UserFlags};
use crate::constants::gateway::events;
use crate::constants::guild::{SYSTEM_CHANNEL_FLAGS_MASK, VerificationLevel};
use crate::constants::limits::{
    GUILD_LIST_PERMISSION_LIMIT, MAX_AFK_TIMEOUT, MAX_DEFAULT_MESSAGE_NOTIFICATIONS,
    MAX_EXPLICIT_CONTENT_FILTER, MAX_GUILD_DESCRIPTION_LENGTH, MAX_GUILD_NAME_LENGTH,
    MAX_GUILDS_PAGE_LIMIT, MAX_GUILDS_PER_USER, MAX_VERIFICATION_LEVEL, MIN_AFK_TIMEOUT,
};
use crate::error::{AppError, ValidationCode};
use crate::models::gateway::events::GuildDeleteEvent;
use crate::models::guild::{Guild, GuildRow};
use crate::models::rest::guilds::{CreateGuildRequest, ListGuildsQuery, UpdateGuildRequest};
use crate::models::role::RolePerm;
use crate::services::{channel, member_service, permission_service, queries, role};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake};
use crate::utils::validation::{reject_control_chars, validate_asset_opt};

use super::gateway::build_full_gateway_guild;
use super::template::prepare_template;

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
    reject_control_chars("name", name)?;

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

        for r in tpl.roles {
            sqlx::query(
                r#"
                INSERT INTO roles (id, guild_id, position, name, color, unicode_emoji, hoist, mentionable, permissions)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&r.id)
            .bind(&guild_id)
            .bind(r.position)
            .bind(&r.name)
            .bind(r.color)
            .bind(&r.unicode_emoji)
            .bind(r.hoist)
            .bind(r.mentionable)
            .bind(r.permissions)
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

        let default_channels = [
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

        for (id, kind, position, parent_id, ch_name) in default_channels {
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

pub async fn get_guild(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<Guild, AppError> {
    let base = sqlx::query_as::<_, GuildRow>("SELECT * FROM guilds WHERE id = ?")
        .bind(guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    member_service::require_member(state, guild_id, user_id).await?;

    let mut roles = role::fetch_guild_roles(state, guild_id).await?;
    let all_channels = channel::fetch_guild_channels(state, guild_id).await?;
    let role_perms: Vec<RolePerm> = roles
        .iter()
        .map(|r| RolePerm {
            id: r.id.clone(),
            permissions: r.permissions,
        })
        .collect();
    if !can_use_threads {
        let mask = !PermissionFlags::THREAD_GATED_BITS.bits();
        for r in &mut roles {
            r.permissions &= mask;
        }
    }
    let member_roles = if base.owner_id == user_id {
        Vec::new()
    } else {
        queries::fetch_member_roles(&state.db, guild_id, user_id).await?
    };
    let mut channels = permission_service::filter_visible_channels_with_context(
        &base.owner_id,
        guild_id,
        user_id,
        &role_perms,
        &member_roles,
        all_channels,
    );
    if !can_use_threads {
        channels.retain(|c| !ChannelType::is_forum_like(c.base.kind));
        for c in &mut channels {
            c.strip_thread_fields();
        }
    }

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

    let current = sqlx::query_as::<_, GuildRow>("SELECT * FROM guilds WHERE id = ? FOR UPDATE")
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
        reject_control_chars("name", trimmed)?;
        sep.push("name = ");
        sep.push_bind_unseparated(trimmed.to_string());
        has_updates = true;
    }

    if let Some(icon) = &payload.icon {
        let value = validate_asset_opt("icon", icon.as_deref())?;
        sep.push("icon = ");
        sep.push_bind_unseparated(value);
        has_updates = true;
    }

    if let Some(banner) = &payload.banner {
        let value = validate_asset_opt("banner", banner.as_deref())?;
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

    let updated_base = sqlx::query_as::<_, GuildRow>("SELECT * FROM guilds WHERE id = ?")
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

pub async fn get_user_guilds(
    state: &AppState,
    user_id: &str,
    can_use_threads: bool,
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

    let rows = qb.build_query_as::<GuildRow>().fetch_all(&state.db).await?;
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
            let mut perms = if is_owner {
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
            if !can_use_threads {
                perms &= !PermissionFlags::THREAD_GATED_BITS.bits();
            }
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
