use std::collections::{HashMap, HashSet};

use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::guild::{DEFAULT_VOICE_BITRATE, DEFAULT_VOICE_CONNECTION_LIMIT};
use crate::constants::limits::{
    MAX_CHANNEL_NAME_LENGTH, MAX_CHANNEL_RATE_LIMIT_PER_USER, MAX_CHANNEL_TOPIC_LENGTH,
    MAX_CHANNELS_PER_CATEGORY, MAX_CHANNELS_PER_GUILD, MAX_CONTENT_WARNING_TEXT_LENGTH,
    MAX_FORUM_TOPIC_LENGTH, MAX_LINK_CHANNEL_URL_LENGTH, MAX_VOICE_BITRATE_CEILING,
    MAX_VOICE_BITRATE_STANDARD,
    MAX_VOICE_CONNECTION_LIMIT, MAX_VOICE_USER_LIMIT, MIN_VOICE_BITRATE,
    MIN_VOICE_CONNECTION_LIMIT,
};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, ChannelRow, PermissionOverwrite,
};
use crate::models::thread::ChannelOrThreadResponse;
use crate::models::guild::{BaseGuild, Guild};
use crate::models::rest::channels::{CreateChannelRequest, UpdateChannelRequest};
use crate::services::{member_service, permission_service, queries};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake};
use crate::utils::validation::{collapse_channel_whitespace, slugify_channel_name};

use super::forum::{
    ensure_unmoderated_tag_if_required, fetch_channel_forum_tags, fetch_guild_forum_tags,
    replace_channel_forum_tags, validate_and_normalize_tags_list,
    validate_default_auto_archive_duration, validate_default_forum_layout,
    validate_default_reaction_input, validate_default_sort_order, validate_default_tag_setting,
    validate_default_thread_rate_limit, validate_forum_flags,
};
use super::overwrites::{
    dispatch_channel_update_with_visibility, propagate_category_overwrites,
    replace_channel_overwrites, validate_and_normalize_overwrites,
};

pub async fn get_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<ChannelOrThreadResponse, AppError> {
    let channel = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    if !can_use_threads
        && (ChannelType::is_forum_like(channel.kind) || ChannelType::is_thread(channel.kind))
    {
        return Err(AppError::unknown_channel());
    }

    if ChannelType::is_thread(channel.kind) {
        let thread =
            crate::services::thread::get_thread(state, channel_id, user_id, can_use_threads)
                .await?;
        return Ok(ChannelOrThreadResponse::Thread(Box::new(thread)));
    }

    permission_service::require_channel_permission(
        state,
        &channel.guild_id,
        &channel.id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await?;

    let overwrites = queries::fetch_channel_overwrites(&state.db, channel_id).await?;
    let tags = if ChannelType::is_forum_like(channel.kind) {
        fetch_channel_forum_tags(&state.db, channel_id).await?
    } else {
        Vec::new()
    };
    let mut resp = ChannelResponse::from_row(channel, overwrites, tags);
    if !can_use_threads {
        resp.strip_thread_fields();
    }
    Ok(ChannelOrThreadResponse::Channel(Box::new(resp)))
}

pub async fn fetch_guild_channels(
    state: &AppState,
    guild_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    let channels = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE guild_id = ? AND type NOT IN (10, 11, 12) ORDER BY position ASC",
        ChannelRow::SELECT_COLS
    ))
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

    let mut tags_by_channel = fetch_guild_forum_tags(&state.db, guild_id).await?;

    Ok(channels
        .into_iter()
        .map(|row| {
            let permission_overwrites = overwrites_by_channel.remove(&row.id).unwrap_or_default();
            let tags = tags_by_channel.remove(&row.id).unwrap_or_default();
            ChannelResponse::from_row(row, permission_overwrites, tags)
        })
        .collect())
}

pub async fn get_guild_channels(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<Vec<ChannelResponse>, AppError> {
    member_service::require_member(state, guild_id, user_id).await?;
    let channels = fetch_guild_channels(state, guild_id).await?;
    let mut visible =
        permission_service::filter_visible_channels(state, guild_id, user_id, channels).await?;
    if !can_use_threads {
        visible.retain(|c| !ChannelType::is_forum_like(c.base.kind));
        for c in &mut visible {
            c.strip_thread_fields();
        }
    }
    Ok(visible)
}

pub async fn create_guild_channel(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    can_use_threads: bool,
    payload: CreateChannelRequest,
) -> Result<ChannelResponse, AppError> {
    if !ChannelType::is_supported_guild_channel(payload.kind)
        || (!can_use_threads && ChannelType::is_forum_like(payload.kind))
    {
        return Err(AppError::validation(
            "type",
            ValidationCode::InvalidFormat,
            "Invalid guild channel type.",
        ));
    }

    let parent_id = payload
        .parent_id
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if let Some(ref pid) = parent_id {
        if payload.kind == ChannelType::GuildCategory as u32 {
            return Err(AppError::validation(
                "parent_id",
                ValidationCode::CategoriesCannotHaveParents,
                "Categories cannot have a parent category.",
            ));
        }
        validate_snowflake("parent_id", pid)?;
    }

    let actor_perms = if let Some(ref pid) = parent_id {
        let parent_meta: Option<(String, u32)> =
            sqlx::query_as("SELECT guild_id, type FROM channels WHERE id = ?")
                .bind(pid)
                .fetch_optional(&state.db)
                .await?;
        let Some((parent_guild, parent_type)) = parent_meta else {
            return Err(AppError::validation(
                "parent_id",
                ValidationCode::InvalidParentChannel,
                "Parent channel does not exist.",
            ));
        };
        if parent_guild != guild_id {
            return Err(AppError::validation(
                "parent_id",
                ValidationCode::ParentChannelNotInGuild,
                "Parent channel must belong to this community.",
            ));
        }
        if parent_type != ChannelType::GuildCategory as u32 {
            return Err(AppError::validation(
                "parent_id",
                ValidationCode::ParentMustBeCategory,
                "Parent channel must be a category.",
            ));
        }
        permission_service::compute_user_channel_permissions(state, guild_id, pid, user_id).await?
    } else {
        permission_service::compute_guild_permissions(state, guild_id, user_id).await?
    };

    if !actor_perms.contains(PermissionFlags::MANAGE_CHANNELS) {
        return Err(AppError::missing_permissions());
    }

    let name = normalize_channel_name(&payload.name, payload.kind)?;
    let topic = validate_topic_opt(payload.topic.flatten().as_deref(), payload.kind)?;
    let url = if payload.kind == ChannelType::GuildLink as u32 {
        validate_link_url(payload.url.flatten().as_deref())?
    } else {
        None
    };

    let (nsfw, nsfw_override) = match payload.nsfw_override {
        Some(Some(val)) => (val, Some(val)),
        Some(None) => (false, None),
        None => match payload.nsfw {
            Some(true) => (true, Some(true)),
            Some(false) | None => (false, None),
        },
    };

    let content_warning_level = if payload.content_warning_level == Some(1) {
        1
    } else {
        0
    };
    let content_warning_text =
        validate_content_warning_text(payload.content_warning_text.flatten().as_deref())?;
    let rate_limit_per_user = validate_rate_limit(payload.rate_limit_per_user.unwrap_or(0))?;

    let is_voice = payload.kind == ChannelType::GuildVoice as u32;
    let bitrate = if is_voice {
        Some(validate_bitrate(
            payload.bitrate.unwrap_or(DEFAULT_VOICE_BITRATE),
        )?)
    } else {
        None
    };
    let user_limit = if is_voice {
        Some(validate_user_limit(payload.user_limit.unwrap_or(0))?)
    } else {
        None
    };
    let voice_connection_limit = if is_voice {
        Some(validate_voice_connection_limit(
            payload
                .voice_connection_limit
                .unwrap_or(DEFAULT_VOICE_CONNECTION_LIMIT),
        )?)
    } else {
        None
    };

    let is_forum_like = ChannelType::is_forum_like(payload.kind);
    let is_forum = payload.kind == ChannelType::GuildForum as u32;
    let flags = if is_forum_like {
        validate_forum_flags(payload.flags.unwrap_or(0), payload.kind)?
    } else {
        0
    };
    let prepared_tags = if is_forum_like {
        let tags = match payload.available_tags {
            Some(ref inputs) => validate_and_normalize_tags_list(inputs, &HashSet::new())?,
            None => Vec::new(),
        };
        ensure_unmoderated_tag_if_required(flags, &tags)?;
        tags
    } else {
        Vec::new()
    };
    let (default_reaction_emoji_id, default_reaction_emoji_name) = if is_forum_like {
        validate_default_reaction_input(payload.default_reaction_emoji.flatten())?
    } else {
        (None, None)
    };
    let default_sort_order = if is_forum_like {
        validate_default_sort_order(payload.default_sort_order.flatten())?
    } else {
        None
    };
    let default_forum_layout = if is_forum {
        validate_default_forum_layout(payload.default_forum_layout.flatten())?
    } else {
        0
    };
    let default_tag_setting = if is_forum_like {
        validate_default_tag_setting(payload.default_tag_setting.flatten().as_deref())?
    } else {
        crate::constants::channel::DEFAULT_FORUM_TAG_SETTING.to_string()
    };
    let (default_auto_archive_duration, default_thread_rate_limit_per_user) =
        if can_use_threads && ChannelType::supports_thread_defaults(payload.kind) {
            (
                validate_default_auto_archive_duration(
                    payload.default_auto_archive_duration.flatten(),
                )?,
                validate_default_thread_rate_limit(
                    payload.default_thread_rate_limit_per_user.flatten(),
                )?,
            )
        } else {
            (None, 0)
        };

    let mut tx = state.db.begin().await?;

    let owner_id: String =
        sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ? FOR UPDATE")
            .bind(guild_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(AppError::unknown_guild)?;

    let total_channels: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_one(&mut *tx)
            .await?;
    if total_channels as u64 >= MAX_CHANNELS_PER_GUILD {
        return Err(AppError::max_guild_channels());
    }

    if let Some(ref pid) = parent_id {
        let cat_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM channels WHERE guild_id = ? AND parent_id = ?",
        )
        .bind(guild_id)
        .bind(pid)
        .fetch_one(&mut *tx)
        .await?;
        if cat_count as u64 >= MAX_CHANNELS_PER_CATEGORY {
            return Err(AppError::max_category_channels());
        }
    }

    let is_owner_or_admin =
        user_id == owner_id || actor_perms.contains(PermissionFlags::ADMINISTRATOR);

    let overwrites_to_insert: Vec<PermissionOverwrite> = match payload.permission_overwrites {
        Some(inputs) => {
            if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
                return Err(AppError::missing_permissions());
            }
            validate_and_normalize_overwrites(
                &mut tx,
                guild_id,
                &inputs,
                actor_perms,
                is_owner_or_admin,
                can_use_threads,
                None,
            )
            .await?
        }
        None => match parent_id {
            Some(ref pid) => queries::fetch_channel_overwrites(&mut *tx, pid).await?,
            None => Vec::new(),
        },
    };

    let max_pos: Option<u32> = if let Some(ref pid) = parent_id {
        sqlx::query_scalar(
            "SELECT MAX(position) FROM channels WHERE guild_id = ? AND parent_id = ?",
        )
        .bind(guild_id)
        .bind(pid)
        .fetch_one(&mut *tx)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT MAX(position) FROM channels WHERE guild_id = ? AND parent_id IS NULL",
        )
        .bind(guild_id)
        .fetch_one(&mut *tx)
        .await?
    };
    let position = max_pos.map_or(0, |p| p.saturating_add(1));

    let channel_id = DISCORD_SNOWFLAKE.generate(None).to_string();

    sqlx::query(
        r#"
        INSERT INTO channels (
            id, type, position, guild_id, parent_id, name, topic, url, nsfw, nsfw_override,
            content_warning_level, content_warning_text, rate_limit_per_user,
            flags, default_auto_archive_duration, default_thread_rate_limit_per_user,
            default_reaction_emoji_id, default_reaction_emoji_name, default_sort_order,
            default_forum_layout, default_tag_setting,
            bitrate, user_limit, voice_connection_limit
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&channel_id)
    .bind(payload.kind)
    .bind(position)
    .bind(guild_id)
    .bind(&parent_id)
    .bind(&name)
    .bind(&topic)
    .bind(&url)
    .bind(nsfw)
    .bind(nsfw_override)
    .bind(content_warning_level)
    .bind(&content_warning_text)
    .bind(rate_limit_per_user)
    .bind(flags)
    .bind(default_auto_archive_duration)
    .bind(default_thread_rate_limit_per_user)
    .bind(&default_reaction_emoji_id)
    .bind(&default_reaction_emoji_name)
    .bind(default_sort_order)
    .bind(default_forum_layout)
    .bind(&default_tag_setting)
    .bind(bitrate)
    .bind(user_limit)
    .bind(voice_connection_limit)
    .execute(&mut *tx)
    .await?;

    replace_channel_overwrites(&mut tx, &channel_id, &overwrites_to_insert).await?;

    if !prepared_tags.is_empty() {
        replace_channel_forum_tags(&mut tx, &channel_id, &HashSet::new(), &prepared_tags).await?;
    }

    let created_row = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(&channel_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let mut response = ChannelResponse::from_row(created_row, overwrites_to_insert, prepared_tags);
    permission_service::dispatch_channel_with_ctx(
        state,
        guild_id,
        &channel_id,
        None,
        events::CHANNEL_CREATE,
        &response,
    )
    .await;

    if !can_use_threads {
        response.strip_thread_fields();
    }

    Ok(response)
}

pub async fn update_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    payload: UpdateChannelRequest,
) -> Result<ChannelOrThreadResponse, AppError> {
    let current = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    if !can_use_threads
        && (ChannelType::is_forum_like(current.kind) || ChannelType::is_thread(current.kind))
    {
        return Err(AppError::unknown_channel());
    }

    if ChannelType::is_thread(current.kind) {
        let thread =
            crate::services::thread::update_thread(state, channel_id, user_id, payload).await?;
        return Ok(ChannelOrThreadResponse::Thread(Box::new(thread)));
    }

    let (perm_ctx, _, actor_perms) = permission_service::require_channel_permission_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_CHANNELS,
    )
    .await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(&current.guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let is_owner_or_admin =
        user_id == owner_id || actor_perms.contains(PermissionFlags::ADMINISTRATOR);

    let new_kind = if let Some(requested_type) = payload.kind {
        if requested_type != current.kind {
            let convertible = (current.kind == ChannelType::GuildText as u32
                || current.kind == ChannelType::GuildAnnouncement as u32)
                && (requested_type == ChannelType::GuildText as u32
                    || requested_type == ChannelType::GuildAnnouncement as u32);
            if !convertible {
                return Err(AppError::channel_type_conversion_not_supported());
            }
        }
        requested_type
    } else {
        current.kind
    };

    let mut tx = state.db.begin().await?;

    let current_locked = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ? FOR UPDATE",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    let old_overwrites = queries::fetch_channel_overwrites(&mut *tx, channel_id).await?;
    let overwrites_changed = payload.permission_overwrites.is_some();

    let new_overwrites = if let Some(inputs) = payload.permission_overwrites {
        if !actor_perms.contains(PermissionFlags::MANAGE_ROLES) {
            return Err(AppError::missing_permissions());
        }
        validate_and_normalize_overwrites(
            &mut tx,
            &current_locked.guild_id,
            &inputs,
            actor_perms,
            is_owner_or_admin,
            can_use_threads,
            Some(&old_overwrites),
        )
        .await?
    } else {
        old_overwrites.clone()
    };

    let old_sessions = if overwrites_changed {
        permission_service::filter_sessions_for_channel_with_ctx(
            state,
            &current_locked.guild_id,
            channel_id,
            perm_ctx.as_ref(),
        )
        .await
    } else {
        Vec::new()
    };

    let mut query = QueryBuilder::<MySql>::new("UPDATE channels SET ");
    let mut sep = query.separated(", ");
    let mut has_col_updates = false;

    if new_kind != current_locked.kind {
        sep.push("type = ");
        sep.push_bind_unseparated(new_kind);
        has_col_updates = true;
    }

    if let Some(raw_name) = &payload.name {
        let normalized = normalize_channel_name(raw_name, new_kind)?;
        sep.push("name = ");
        sep.push_bind_unseparated(normalized);
        has_col_updates = true;
    }

    if let Some(pos) = payload.position {
        sep.push("position = ");
        sep.push_bind_unseparated(pos);
        has_col_updates = true;
    }

    if let Some(topic) = &payload.topic {
        let val = validate_topic_opt(topic.as_deref(), new_kind)?;
        sep.push("topic = ");
        sep.push_bind_unseparated(val);
        has_col_updates = true;
    }

    if let Some(url) = &payload.url
        && new_kind == ChannelType::GuildLink as u32
    {
        let val = validate_link_url(url.as_deref())?;
        sep.push("url = ");
        sep.push_bind_unseparated(val);
        has_col_updates = true;
    }

    if let Some(parent_opt) = &payload.parent_id {
        let next_parent = parent_opt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        if let Some(ref pid) = next_parent {
            if current_locked.kind == ChannelType::GuildCategory as u32 {
                return Err(AppError::validation(
                    "parent_id",
                    ValidationCode::CategoriesCannotHaveParents,
                    "Categories cannot have a parent category.",
                ));
            }
            validate_snowflake("parent_id", pid)?;
            let parent_meta: Option<(String, u32)> =
                sqlx::query_as("SELECT guild_id, type FROM channels WHERE id = ?")
                    .bind(pid)
                    .fetch_optional(&mut *tx)
                    .await?;
            match parent_meta {
                None => {
                    return Err(AppError::validation(
                        "parent_id",
                        ValidationCode::InvalidParentChannel,
                        "Parent channel does not exist.",
                    ));
                }
                Some((pg, _)) if pg != current_locked.guild_id => {
                    return Err(AppError::validation(
                        "parent_id",
                        ValidationCode::ParentChannelNotInGuild,
                        "Parent channel must belong to this community.",
                    ));
                }
                Some((_, pt)) if pt != ChannelType::GuildCategory as u32 => {
                    return Err(AppError::validation(
                        "parent_id",
                        ValidationCode::ParentMustBeCategory,
                        "Parent channel must be a category.",
                    ));
                }
                _ => {}
            }
            if current_locked.parent_id.as_deref() != Some(pid.as_str()) {
                let target_perms = permission_service::compute_user_channel_permissions_conn(
                    &mut tx,
                    &current_locked.guild_id,
                    pid,
                    user_id,
                )
                .await?;
                if !target_perms
                    .contains(PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_CHANNELS)
                {
                    return Err(AppError::missing_permissions());
                }
                let cat_count: i64 = sqlx::query_scalar(
                    "SELECT COUNT(*) FROM channels WHERE guild_id = ? AND parent_id = ?",
                )
                .bind(&current_locked.guild_id)
                .bind(pid)
                .fetch_one(&mut *tx)
                .await?;
                if cat_count as u64 >= MAX_CHANNELS_PER_CATEGORY {
                    return Err(AppError::max_category_channels());
                }
            }
        }
        sep.push("parent_id = ");
        sep.push_bind_unseparated(next_parent);
        has_col_updates = true;
    }

    if new_kind == ChannelType::GuildVoice as u32 {
        if let Some(bitrate_opt) = payload.bitrate {
            let val = validate_bitrate(bitrate_opt.unwrap_or(DEFAULT_VOICE_BITRATE))?;
            sep.push("bitrate = ");
            sep.push_bind_unseparated(Some(val));
            has_col_updates = true;
        }
        if let Some(limit_opt) = payload.user_limit {
            let val = validate_user_limit(limit_opt.unwrap_or(0))?;
            sep.push("user_limit = ");
            sep.push_bind_unseparated(Some(val));
            has_col_updates = true;
        }
        if let Some(conn_limit_opt) = payload.voice_connection_limit {
            let val = validate_voice_connection_limit(
                conn_limit_opt.unwrap_or(DEFAULT_VOICE_CONNECTION_LIMIT),
            )?;
            sep.push("voice_connection_limit = ");
            sep.push_bind_unseparated(Some(val));
            has_col_updates = true;
        }
    }

    if let Some(rate_opt) = payload.rate_limit_per_user {
        let val = validate_rate_limit(rate_opt.unwrap_or(0))?;
        sep.push("rate_limit_per_user = ");
        sep.push_bind_unseparated(val);
        has_col_updates = true;
    }

    match payload.nsfw_override {
        Some(Some(val)) => {
            sep.push("nsfw_override = ");
            sep.push_bind_unseparated(Some(val));
            sep.push("nsfw = ");
            sep.push_bind_unseparated(val);
            has_col_updates = true;
        }
        Some(None) => {
            sep.push("nsfw_override = ");
            sep.push_bind_unseparated(Option::<bool>::None);
            sep.push("nsfw = ");
            sep.push_bind_unseparated(false);
            has_col_updates = true;
        }
        None => {
            if let Some(val) = payload.nsfw {
                sep.push("nsfw_override = ");
                sep.push_bind_unseparated(Some(val));
                sep.push("nsfw = ");
                sep.push_bind_unseparated(val);
                has_col_updates = true;
            }
        }
    }

    if let Some(level) = payload.content_warning_level {
        let stored = if level == 1 { 1u8 } else { 0u8 };
        sep.push("content_warning_level = ");
        sep.push_bind_unseparated(stored);
        has_col_updates = true;
    }

    if let Some(cw_text) = &payload.content_warning_text {
        let val = validate_content_warning_text(cw_text.as_deref())?;
        sep.push("content_warning_text = ");
        sep.push_bind_unseparated(val);
        has_col_updates = true;
    }

    let is_forum_like = ChannelType::is_forum_like(new_kind);
    let is_forum = new_kind == ChannelType::GuildForum as u32;

    let next_flags = if is_forum_like {
        if let Some(flags_val) = payload.flags {
            let validated = validate_forum_flags(flags_val, new_kind)?;
            sep.push("flags = ");
            sep.push_bind_unseparated(validated);
            has_col_updates = true;
            validated
        } else {
            current_locked.flags
        }
    } else {
        current_locked.flags
    };

    let existing_tags = if is_forum_like {
        fetch_channel_forum_tags(&mut *tx, channel_id).await?
    } else {
        Vec::new()
    };
    let existing_tag_ids: HashSet<String> = existing_tags.iter().map(|t| t.id.clone()).collect();
    let tags_changed = is_forum_like && payload.available_tags.is_some();
    let new_tags = if is_forum_like {
        let candidate = match &payload.available_tags {
            Some(inputs) => validate_and_normalize_tags_list(inputs, &existing_tag_ids)?,
            None => existing_tags,
        };
        if payload.flags.is_some() || payload.available_tags.is_some() {
            ensure_unmoderated_tag_if_required(next_flags, &candidate)?;
        }
        candidate
    } else {
        Vec::new()
    };

    if is_forum_like {
        if let Some(reaction_opt) = payload.default_reaction_emoji {
            let (eid, ename) = validate_default_reaction_input(reaction_opt)?;
            sep.push("default_reaction_emoji_id = ");
            sep.push_bind_unseparated(eid);
            sep.push("default_reaction_emoji_name = ");
            sep.push_bind_unseparated(ename);
            has_col_updates = true;
        }
        if let Some(sort_opt) = payload.default_sort_order {
            let val = validate_default_sort_order(sort_opt)?;
            sep.push("default_sort_order = ");
            sep.push_bind_unseparated(val);
            has_col_updates = true;
        }
        if is_forum && let Some(layout_opt) = payload.default_forum_layout {
            let val = validate_default_forum_layout(layout_opt)?;
            sep.push("default_forum_layout = ");
            sep.push_bind_unseparated(val);
            has_col_updates = true;
        }
        if let Some(tag_setting_opt) = &payload.default_tag_setting {
            let val = validate_default_tag_setting(tag_setting_opt.as_deref())?;
            sep.push("default_tag_setting = ");
            sep.push_bind_unseparated(val);
            has_col_updates = true;
        }
    }

    if can_use_threads && ChannelType::supports_thread_defaults(new_kind) {
        if let Some(auto_opt) = payload.default_auto_archive_duration {
            let val = validate_default_auto_archive_duration(auto_opt)?;
            sep.push("default_auto_archive_duration = ");
            sep.push_bind_unseparated(val);
            has_col_updates = true;
        }
        if let Some(thread_rate_opt) = payload.default_thread_rate_limit_per_user {
            let val = validate_default_thread_rate_limit(thread_rate_opt)?;
            sep.push("default_thread_rate_limit_per_user = ");
            sep.push_bind_unseparated(val);
            has_col_updates = true;
        }
    }

    if has_col_updates {
        query.push(" WHERE id = ");
        query.push_bind(channel_id);
        query.build().execute(&mut *tx).await?;
    }

    let mut synced_children = Vec::new();
    if overwrites_changed {
        replace_channel_overwrites(&mut tx, channel_id, &new_overwrites).await?;
        if current_locked.kind == ChannelType::GuildCategory as u32 {
            synced_children = propagate_category_overwrites(
                &mut tx,
                &current_locked.guild_id,
                channel_id,
                &old_overwrites,
                &new_overwrites,
            )
            .await?;
        }
    }

    if tags_changed {
        replace_channel_forum_tags(&mut tx, channel_id, &existing_tag_ids, &new_tags).await?;
    }

    let updated_row = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let mut updated = ChannelResponse::from_row(updated_row, new_overwrites, new_tags);

    if overwrites_changed {
        dispatch_channel_update_with_visibility(
            state,
            &current_locked.guild_id,
            channel_id,
            old_sessions,
            &updated,
        )
        .await;

        for child in synced_children {
            permission_service::dispatch_channel_with_ctx(
                state,
                &current_locked.guild_id,
                &child.base.id,
                None,
                events::CHANNEL_UPDATE,
                &child,
            )
            .await;
        }

        crate::gateway::handlers::lazy_request::refresh_guild_member_lists(
            state,
            &current_locked.guild_id,
        )
        .await;
    } else if has_col_updates || tags_changed {
        permission_service::dispatch_channel_with_ctx(
            state,
            &current_locked.guild_id,
            channel_id,
            perm_ctx.as_ref(),
            events::CHANNEL_UPDATE,
            &updated,
        )
        .await;
    }

    if !can_use_threads {
        updated.strip_thread_fields();
    }

    Ok(ChannelOrThreadResponse::Channel(Box::new(updated)))
}

pub async fn delete_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    if !can_use_threads
        && (ChannelType::is_forum_like(row.kind) || ChannelType::is_thread(row.kind))
    {
        return Err(AppError::unknown_channel());
    }

    if ChannelType::is_thread(row.kind) {
        return crate::services::thread::delete_thread(state, channel_id, user_id).await;
    }

    let (perm_ctx, _, _) = permission_service::require_channel_permission_with_ctx(
        state,
        &row.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_CHANNELS,
    )
    .await?;

    let overwrites = queries::fetch_channel_overwrites(&state.db, channel_id).await?;
    let tags = if ChannelType::is_forum_like(row.kind) {
        fetch_channel_forum_tags(&state.db, channel_id).await?
    } else {
        Vec::new()
    };
    let viewer_sessions = permission_service::filter_sessions_for_channel_with_ctx(
        state,
        &row.guild_id,
        channel_id,
        perm_ctx.as_ref(),
    )
    .await;

    let mut tx = state.db.begin().await?;

    let child_ids: Vec<String> = if row.kind == ChannelType::GuildCategory as u32 {
        sqlx::query_scalar(
            "SELECT id FROM channels WHERE guild_id = ? AND parent_id = ? AND type NOT IN (10, 11, 12)",
        )
        .bind(&row.guild_id)
        .bind(channel_id)
        .fetch_all(&mut *tx)
        .await?
    } else {
        Vec::new()
    };

    if row.kind != ChannelType::GuildCategory as u32 {
        sqlx::query(
            "DELETE FROM channels WHERE guild_id = ? AND parent_id = ? AND type IN (10, 11, 12)",
        )
        .bind(&row.guild_id)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;
    }

    let guild_update_res = sqlx::query(
        r#"
        UPDATE guilds
        SET system_channel_id = IF(system_channel_id = ?, NULL, system_channel_id),
            rules_channel_id = IF(rules_channel_id = ?, NULL, rules_channel_id),
            afk_channel_id = IF(afk_channel_id = ?, NULL, afk_channel_id)
        WHERE id = ?
          AND (system_channel_id = ? OR rules_channel_id = ? OR afk_channel_id = ?)
        "#,
    )
    .bind(channel_id)
    .bind(channel_id)
    .bind(channel_id)
    .bind(&row.guild_id)
    .bind(channel_id)
    .bind(channel_id)
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;

    let updated_guild_base = if guild_update_res.rows_affected() > 0 {
        sqlx::query_as::<_, BaseGuild>("SELECT * FROM guilds WHERE id = ?")
            .bind(&row.guild_id)
            .fetch_optional(&mut *tx)
            .await?
    } else {
        None
    };

    let res = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::unknown_channel());
    }

    let updated_children = if child_ids.is_empty() {
        Vec::new()
    } else {
        let mut cqb = QueryBuilder::<MySql>::new(format!(
            "SELECT {} FROM channels WHERE id IN (",
            ChannelRow::SELECT_COLS
        ));
        let mut sep = cqb.separated(", ");
        for cid in &child_ids {
            sep.push_bind(cid);
        }
        cqb.push(")");
        let child_rows = cqb
            .build_query_as::<ChannelRow>()
            .fetch_all(&mut *tx)
            .await?;

        let mut oqb = QueryBuilder::<MySql>::new(
            "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id IN (",
        );
        let mut sep = oqb.separated(", ");
        for cid in &child_ids {
            sep.push_bind(cid);
        }
        oqb.push(")");
        let child_ow_rows = oqb
            .build_query_as::<ChannelOverwriteRow>()
            .fetch_all(&mut *tx)
            .await?;

        let mut ow_by_ch: HashMap<String, Vec<PermissionOverwrite>> = HashMap::new();
        for ow in child_ow_rows {
            ow_by_ch
                .entry(ow.channel_id.clone())
                .or_default()
                .push(PermissionOverwrite::from(ow));
        }

        let mut tags_by_ch = fetch_guild_forum_tags(&mut *tx, &row.guild_id).await?;

        child_rows
            .into_iter()
            .map(|c| {
                let ows = ow_by_ch.remove(&c.id).unwrap_or_default();
                let c_tags = tags_by_ch.remove(&c.id).unwrap_or_default();
                ChannelResponse::from_row(c, ows, c_tags)
            })
            .collect()
    };

    tx.commit().await?;

    for (session, sub_cid, _) in state.hub.guild_member_list_subscribers(&row.guild_id) {
        if sub_cid == channel_id {
            session.remove_member_list_subscription(&row.guild_id);
        }
    }

    let guild_id = row.guild_id.clone();
    let deleted_channel = ChannelResponse::from_row(row, overwrites, tags);
    state.dispatch_to_sessions(&viewer_sessions, events::CHANNEL_DELETE, &deleted_channel);

    for child in updated_children {
        permission_service::dispatch_channel_with_ctx(
            state,
            &guild_id,
            &child.base.id,
            None,
            events::CHANNEL_UPDATE,
            &child,
        )
        .await;
    }

    if let Some(base) = updated_guild_base {
        let updated_guild = Guild::from(base);
        state
            .dispatch_guild(events::GUILD_UPDATE, &updated_guild, &guild_id)
            .await;
    }

    Ok(())
}

fn normalize_channel_name(raw: &str, kind: u32) -> Result<String, AppError> {
    let collapsed = collapse_channel_whitespace(raw);
    let trimmed = collapsed.trim();
    let normalized = slugify_channel_name(trimmed, kind);

    if normalized.is_empty() {
        return Err(AppError::validation(
            "name",
            ValidationCode::NameEmptyAfterNormalization,
            "Channel name cannot be empty.",
        ));
    }
    if normalized.chars().count() > MAX_CHANNEL_NAME_LENGTH {
        return Err(AppError::validation(
            "name",
            ValidationCode::StringLengthInvalid,
            "Channel name must be between 1 and 100 characters.",
        ));
    }

    Ok(normalized)
}

fn validate_topic_opt(raw: Option<&str>, kind: u32) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let max_len = if ChannelType::is_forum_like(kind) {
        MAX_FORUM_TOPIC_LENGTH
    } else {
        MAX_CHANNEL_TOPIC_LENGTH
    };
    if trimmed.chars().count() > max_len {
        return Err(AppError::validation(
            "topic",
            ValidationCode::ContentExceedsMaxLength,
            format!("Channel topic must not exceed {max_len} characters."),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_content_warning_text(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MAX_CONTENT_WARNING_TEXT_LENGTH {
        return Err(AppError::validation(
            "content_warning_text",
            ValidationCode::ContentExceedsMaxLength,
            "Content warning text must not exceed 200 characters.",
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_link_url(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MAX_LINK_CHANNEL_URL_LENGTH {
        return Err(AppError::validation(
            "url",
            ValidationCode::UrlLengthInvalid,
            "URL must not exceed 2048 characters.",
        ));
    }
    if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(AppError::validation(
            "url",
            ValidationCode::InvalidUrlFormat,
            "Invalid URL format.",
        ));
    }
    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .ok_or_else(|| {
            AppError::validation(
                "url",
                ValidationCode::InvalidUrlFormat,
                "URL must use http or https.",
            )
        })?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(AppError::validation(
            "url",
            ValidationCode::InvalidUrlFormat,
            "Invalid URL authority.",
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn validate_rate_limit(val: u32) -> Result<u32, AppError> {
    if val > MAX_CHANNEL_RATE_LIMIT_PER_USER {
        return Err(AppError::validation(
            "rate_limit_per_user",
            ValidationCode::ValueMustBeIntegerInRange,
            "Slowmode must be between 0 and 21600 seconds.",
        ));
    }
    Ok(val)
}

fn validate_bitrate(val: u32) -> Result<u32, AppError> {
    if !(MIN_VOICE_BITRATE..=MAX_VOICE_BITRATE_CEILING).contains(&val) {
        return Err(AppError::validation(
            "bitrate",
            ValidationCode::ValueMustBeIntegerInRange,
            "Bitrate must be between 8000 and 384000.",
        ));
    }
    Ok(val.min(MAX_VOICE_BITRATE_STANDARD))
}

fn validate_user_limit(val: u32) -> Result<u32, AppError> {
    if val > MAX_VOICE_USER_LIMIT {
        return Err(AppError::validation(
            "user_limit",
            ValidationCode::ValueMustBeIntegerInRange,
            "User limit must be between 0 and 99.",
        ));
    }
    Ok(val)
}

fn validate_voice_connection_limit(val: u32) -> Result<u32, AppError> {
    if !(MIN_VOICE_CONNECTION_LIMIT..=MAX_VOICE_CONNECTION_LIMIT).contains(&val) {
        return Err(AppError::validation(
            "voice_connection_limit",
            ValidationCode::ValueMustBeIntegerInRange,
            "Voice connection limit must be between 1 and 100.",
        ));
    }
    Ok(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_channel_name_handles_text_and_voice() {
        assert_eq!(
            normalize_channel_name("  Hello World! ", ChannelType::GuildText as u32).unwrap(),
            "hello-world"
        );
        assert_eq!(
            normalize_channel_name("  Voice Lounge! ", ChannelType::GuildVoice as u32).unwrap(),
            "Voice Lounge!"
        );
        assert!(normalize_channel_name("!!!", ChannelType::GuildText as u32).is_err());
    }

    #[test]
    fn validate_topic_opt_uses_4096_for_forum_and_media() {
        let long_topic = "a".repeat(2000);
        assert!(validate_topic_opt(Some(&long_topic), ChannelType::GuildText as u32).is_err());
        assert!(validate_topic_opt(Some(&long_topic), ChannelType::GuildForum as u32).is_ok());
        assert!(validate_topic_opt(Some(&long_topic), ChannelType::GuildMedia as u32).is_ok());
    }

    #[test]
    fn validate_link_url_rejects_credentials_and_bad_schemes() {
        assert_eq!(
            validate_link_url(Some("https://example.com/docs")).unwrap(),
            Some("https://example.com/docs".to_string())
        );
        assert!(validate_link_url(Some("ftp://example.com")).is_err());
        assert!(validate_link_url(Some("https://user:pass@example.com")).is_err());
        assert_eq!(validate_link_url(Some("   ")).unwrap(), None);
    }
}
