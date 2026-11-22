use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::guild::{
    DEFAULT_VOICE_BITRATE, DEFAULT_VOICE_CONNECTION_LIMIT, MAX_VOICE_BITRATE_CEILING,
    MAX_VOICE_BITRATE_STANDARD, MAX_VOICE_USER_LIMIT, MAX_VOICE_CONNECTION_LIMIT,
    MIN_VOICE_BITRATE, MIN_VOICE_CONNECTION_LIMIT,
};
use crate::constants::limits::{
    MAX_CHANNEL_NAME_LENGTH, MAX_CHANNEL_OVERWRITES, MAX_CHANNEL_RATE_LIMIT_PER_USER,
    MAX_CHANNEL_TOPIC_LENGTH, MAX_CHANNELS_PER_CATEGORY, MAX_CHANNELS_PER_GUILD,
    MAX_CONTENT_WARNING_TEXT_LENGTH, MAX_LINK_CHANNEL_URL_LENGTH,
};
use crate::error::{AppError, ValidationCode};
use crate::gateway::session::GatewaySession;
use crate::models::channel::{
    ChannelOverwriteRow, ChannelResponse, ChannelRow, ChannelType, PermissionOverwrite,
};
use crate::models::permission::OverwriteType;
use crate::models::rest::channels::{
    ChannelPositionUpdate, CreateChannelRequest, PermissionOverwriteInput,
    SetPermissionOverwriteRequest, UpdateChannelRequest,
};
use crate::services::{member_service, permission_service};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake};

pub async fn get_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> Result<ChannelResponse, AppError> {
    let channel = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    permission_service::require_channel_permission(
        state,
        &channel.guild_id,
        &channel.id,
        user_id,
        PermissionFlags::VIEW_CHANNEL,
    )
    .await?;

    let overwrites = fetch_channel_overwrites(&state.db, channel_id).await?;
    Ok(ChannelResponse::from_row(channel, overwrites))
}

pub async fn fetch_guild_channels(
    state: &AppState,
    guild_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    let channels = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE guild_id = ? ORDER BY position ASC",
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

    Ok(channels
        .into_iter()
        .map(|row| {
            let permission_overwrites = overwrites_by_channel.remove(&row.id).unwrap_or_default();
            ChannelResponse::from_row(row, permission_overwrites)
        })
        .collect())
}

pub async fn get_guild_channels(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
) -> Result<Vec<ChannelResponse>, AppError> {
    member_service::require_member(state, guild_id, user_id).await?;
    let channels = fetch_guild_channels(state, guild_id).await?;
    permission_service::filter_visible_channels(state, guild_id, user_id, channels).await
}

pub async fn create_guild_channel(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: CreateChannelRequest,
) -> Result<ChannelResponse, AppError> {
    if !is_supported_guild_channel_type(payload.kind) {
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
    let topic = validate_topic_opt(payload.topic.flatten().as_deref())?;
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

    let mut tx = state.db.begin().await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ? FOR UPDATE")
        .bind(guild_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let total_channels: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE guild_id = ?")
        .bind(guild_id)
        .fetch_one(&mut *tx)
        .await?;
    if total_channels as u64 >= MAX_CHANNELS_PER_GUILD {
        return Err(AppError::max_guild_channels());
    }

    if let Some(ref pid) = parent_id {
        let cat_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM channels WHERE guild_id = ? AND parent_id = ?")
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
                None,
            )
            .await?
        }
        None => match parent_id {
            Some(ref pid) => fetch_channel_overwrites(&mut *tx, pid).await?,
            None => Vec::new(),
        },
    };

    let max_pos: Option<u32> = if let Some(ref pid) = parent_id {
        sqlx::query_scalar("SELECT MAX(position) FROM channels WHERE guild_id = ? AND parent_id = ?")
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
            bitrate, user_limit, voice_connection_limit
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
    .bind(bitrate)
    .bind(user_limit)
    .bind(voice_connection_limit)
    .execute(&mut *tx)
    .await?;

    replace_channel_overwrites(&mut tx, &channel_id, &overwrites_to_insert).await?;

    let created_row = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(&channel_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let response = ChannelResponse::from_row(created_row, overwrites_to_insert);
    permission_service::dispatch_channel_with_ctx(
        state,
        guild_id,
        &channel_id,
        None,
        events::CHANNEL_CREATE,
        &response,
    )
    .await;

    Ok(response)
}

pub async fn update_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    payload: UpdateChannelRequest,
) -> Result<ChannelResponse, AppError> {
    let current = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

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

    let old_overwrites = fetch_channel_overwrites(&mut *tx, channel_id).await?;
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
        let val = validate_topic_opt(topic.as_deref())?;
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

    let updated_row = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    let updated = ChannelResponse::from_row(updated_row, new_overwrites);

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
    } else if has_col_updates {
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

    Ok(updated)
}

pub async fn delete_channel(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let row = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    let (perm_ctx, _, _) = permission_service::require_channel_permission_with_ctx(
        state,
        &row.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_CHANNELS,
    )
    .await?;

    let overwrites = fetch_channel_overwrites(&state.db, channel_id).await?;
    let viewer_sessions = permission_service::filter_sessions_for_channel_with_ctx(
        state,
        &row.guild_id,
        channel_id,
        perm_ctx.as_ref(),
    )
    .await;

    let mut tx = state.db.begin().await?;

    let child_ids: Vec<String> = if row.kind == ChannelType::GuildCategory as u32 {
        sqlx::query_scalar("SELECT id FROM channels WHERE guild_id = ? AND parent_id = ?")
            .bind(&row.guild_id)
            .bind(channel_id)
            .fetch_all(&mut *tx)
            .await?
    } else {
        Vec::new()
    };

    sqlx::query(
        r#"
        UPDATE guilds
        SET system_channel_id = IF(system_channel_id = ?, NULL, system_channel_id),
            rules_channel_id = IF(rules_channel_id = ?, NULL, rules_channel_id),
            afk_channel_id = IF(afk_channel_id = ?, NULL, afk_channel_id)
        WHERE id = ?
        "#,
    )
    .bind(channel_id)
    .bind(channel_id)
    .bind(channel_id)
    .bind(&row.guild_id)
    .execute(&mut *tx)
    .await?;

    let res = sqlx::query("DELETE FROM channels WHERE id = ?")
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    if res.rows_affected() == 0 {
        return Err(AppError::unknown_channel());
    }

    let mut updated_children = Vec::with_capacity(child_ids.len());
    for cid in child_ids {
        if let Some(child_row) = sqlx::query_as::<_, ChannelRow>(&format!(
            "SELECT {} FROM channels WHERE id = ?",
            ChannelRow::SELECT_COLS
        ))
        .bind(&cid)
        .fetch_optional(&mut *tx)
        .await?
        {
            let child_ows = fetch_channel_overwrites(&mut *tx, &cid).await?;
            updated_children.push(ChannelResponse::from_row(child_row, child_ows));
        }
    }

    tx.commit().await?;

    for (session, sub_cid, _) in state.hub.guild_member_list_subscribers(&row.guild_id) {
        if sub_cid == channel_id {
            session.remove_member_list_subscription(&row.guild_id);
        }
    }

    let guild_id = row.guild_id.clone();
    let deleted_channel = ChannelResponse::from_row(row, overwrites);
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

    Ok(())
}

pub async fn set_permission_overwrite(
    state: &AppState,
    channel_id: &str,
    overwrite_id: &str,
    user_id: &str,
    payload: SetPermissionOverwriteRequest,
) -> Result<(), AppError> {
    let current = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    let (perm_ctx, _, actor_perms) = permission_service::require_channel_permission_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_ROLES,
    )
    .await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(&current.guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let is_owner_or_admin =
        user_id == owner_id || actor_perms.contains(PermissionFlags::ADMINISTRATOR);

    validate_snowflake("overwrite_id", overwrite_id)?;

    let mut tx = state.db.begin().await?;
    let old_overwrites = fetch_channel_overwrites(&mut *tx, channel_id).await?;

    let mut next_inputs: Vec<PermissionOverwriteInput> = old_overwrites
        .iter()
        .filter(|o| o.id != overwrite_id)
        .map(|o| PermissionOverwriteInput {
            id: o.id.clone(),
            kind: o.kind,
            allow: Some(o.allow),
            deny: Some(o.deny),
        })
        .collect();
    next_inputs.push(PermissionOverwriteInput {
        id: overwrite_id.to_string(),
        kind: payload.kind,
        allow: payload.allow,
        deny: payload.deny,
    });

    let new_overwrites = validate_and_normalize_overwrites(
        &mut tx,
        &current.guild_id,
        &next_inputs,
        actor_perms,
        is_owner_or_admin,
        Some(&old_overwrites),
    )
    .await?;

    let old_sessions = permission_service::filter_sessions_for_channel_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        perm_ctx.as_ref(),
    )
    .await;

    replace_channel_overwrites(&mut tx, channel_id, &new_overwrites).await?;

    let synced_children = if current.kind == ChannelType::GuildCategory as u32 {
        propagate_category_overwrites(
            &mut tx,
            &current.guild_id,
            channel_id,
            &old_overwrites,
            &new_overwrites,
        )
        .await?
    } else {
        Vec::new()
    };

    tx.commit().await?;

    let guild_id = current.guild_id.clone();
    let updated = ChannelResponse::from_row(current, new_overwrites);
    dispatch_channel_update_with_visibility(state, &guild_id, channel_id, old_sessions, &updated)
        .await;

    for child in synced_children {
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

    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, &guild_id).await;

    Ok(())
}

pub async fn delete_permission_overwrite(
    state: &AppState,
    channel_id: &str,
    overwrite_id: &str,
    user_id: &str,
) -> Result<(), AppError> {
    let current = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(AppError::unknown_channel)?;

    let (perm_ctx, _, actor_perms) = permission_service::require_channel_permission_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_ROLES,
    )
    .await?;

    let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
        .bind(&current.guild_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(AppError::unknown_guild)?;

    let is_owner_or_admin =
        user_id == owner_id || actor_perms.contains(PermissionFlags::ADMINISTRATOR);

    let mut tx = state.db.begin().await?;
    let old_overwrites = fetch_channel_overwrites(&mut *tx, channel_id).await?;

    let Some(target_ow) = old_overwrites.iter().find(|o| o.id == overwrite_id) else {
        tx.commit().await?;
        return Ok(());
    };

    if !is_owner_or_admin && (target_ow.deny & !actor_perms.bits()) != 0 {
        return Err(AppError::missing_permissions());
    }

    let new_overwrites: Vec<PermissionOverwrite> = old_overwrites
        .iter()
        .filter(|o| o.id != overwrite_id)
        .cloned()
        .collect();

    let old_sessions = permission_service::filter_sessions_for_channel_with_ctx(
        state,
        &current.guild_id,
        channel_id,
        perm_ctx.as_ref(),
    )
    .await;

    sqlx::query("DELETE FROM channel_permission_overwrites WHERE channel_id = ? AND target_id = ?")
        .bind(channel_id)
        .bind(overwrite_id)
        .execute(&mut *tx)
        .await?;

    let synced_children = if current.kind == ChannelType::GuildCategory as u32 {
        propagate_category_overwrites(
            &mut tx,
            &current.guild_id,
            channel_id,
            &old_overwrites,
            &new_overwrites,
        )
        .await?
    } else {
        Vec::new()
    };

    tx.commit().await?;

    let guild_id = current.guild_id.clone();
    let updated = ChannelResponse::from_row(current, new_overwrites);
    dispatch_channel_update_with_visibility(state, &guild_id, channel_id, old_sessions, &updated)
        .await;

    for child in synced_children {
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

    crate::gateway::handlers::lazy_request::refresh_guild_member_lists(state, &guild_id).await;

    Ok(())
}

pub async fn modify_guild_channel_positions(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    payload: Vec<ChannelPositionUpdate>,
) -> Result<(), AppError> {
    let perms = permission_service::compute_guild_permissions(state, guild_id, user_id).await?;
    if !perms.contains(PermissionFlags::MANAGE_CHANNELS) {
        return Err(AppError::missing_permissions());
    }

    if payload.is_empty() {
        return Ok(());
    }

    let mut tx = state.db.begin().await?;

    let rows = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE guild_id = ? ORDER BY position ASC, CAST(id AS UNSIGNED) ASC FOR UPDATE",
        ChannelRow::SELECT_COLS
    ))
    .bind(guild_id)
    .fetch_all(&mut *tx)
    .await?;

    let mut by_id: HashMap<String, ChannelRow> =
        rows.into_iter().map(|r| (r.id.clone(), r)).collect();

    for entry in &payload {
        let Some(ch) = by_id.get(&entry.id) else {
            return Err(AppError::validation(
                "id",
                ValidationCode::ChannelNotFound,
                "Channel does not exist in this community.",
            ));
        };
        if let Some(Some(ref pid)) = entry.parent_id {
            if ch.kind == ChannelType::GuildCategory as u32 {
                return Err(AppError::validation(
                    "parent_id",
                    ValidationCode::CategoriesCannotHaveParents,
                    "Categories cannot have a parent category.",
                ));
            }
            let Some(parent) = by_id.get(pid) else {
                return Err(AppError::validation(
                    "parent_id",
                    ValidationCode::InvalidParentChannel,
                    "Parent channel does not exist in this community.",
                ));
            };
            if parent.kind != ChannelType::GuildCategory as u32 {
                return Err(AppError::validation(
                    "parent_id",
                    ValidationCode::ParentMustBeCategory,
                    "Parent channel must be a category.",
                ));
            }
        }
        if let Some(Some(ref sib_id)) = entry.preceding_sibling_id {
            if sib_id == &entry.id {
                return Err(AppError::validation(
                    "preceding_sibling_id",
                    ValidationCode::CannotPositionChannelRelativeToItself,
                    "Cannot position a channel relative to itself.",
                ));
            }
            if !by_id.contains_key(sib_id) {
                return Err(AppError::validation(
                    "preceding_sibling_id",
                    ValidationCode::ChannelNotFound,
                    "Preceding channel does not exist in this community.",
                ));
            }
        }
    }

    let original_state: HashMap<String, (Option<String>, u32)> = by_id
        .iter()
        .map(|(id, ch)| (id.clone(), (ch.parent_id.clone(), ch.position)))
        .collect();

    let mut lock_copies: Vec<(String, String)> = Vec::new();
    let mut dirty_ids: HashSet<String> = HashSet::new();

    for entry in payload {
        let Some(current_ch) = by_id.get(&entry.id).cloned() else {
            continue;
        };
        let old_parent = current_ch.parent_id.clone();
        let new_parent = match entry.parent_id {
            Some(opt) => opt
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            None => old_parent.clone(),
        };

        if let Some(Some(ref sib_id)) = entry.preceding_sibling_id
            && by_id
                .get(sib_id)
                .is_none_or(|sib| sib.parent_id != new_parent)
        {
            return Err(AppError::validation(
                "preceding_sibling_id",
                ValidationCode::PrecedingChannelMustShareParent,
                "Preceding sibling must share the destination parent.",
            ));
        }

        if new_parent != old_parent
            && let Some(ref pid) = new_parent
        {
            let count = by_id
                .values()
                .filter(|c| c.id != entry.id && c.parent_id.as_deref() == Some(pid.as_str()))
                .count() as u64;
            if count >= MAX_CHANNELS_PER_CATEGORY {
                return Err(AppError::max_category_channels());
            }
            if entry.lock_permissions == Some(true) {
                lock_copies.push((entry.id.clone(), pid.clone()));
            }
        }

        if let Some(ch) = by_id.get_mut(&entry.id) {
            ch.parent_id = new_parent.clone();
        }

        let mut siblings: Vec<String> = by_id
            .values()
            .filter(|c| c.id != entry.id && c.parent_id == new_parent)
            .map(|c| (c.position, c.id.clone()))
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(_, id)| id)
            .collect();
        siblings.sort_by_key(|id| by_id.get(id).map_or(0, |c| c.position));

        let insert_idx = if let Some(sib_opt) = entry.preceding_sibling_id {
            match sib_opt {
                None => 0,
                Some(sib_id) => siblings
                    .iter()
                    .position(|id| id == &sib_id)
                    .map_or(siblings.len(), |idx| idx + 1),
            }
        } else if let Some(pos) = entry.position {
            (pos as usize).min(siblings.len())
        } else if current_ch.kind == ChannelType::GuildVoice as u32 {
            siblings.len()
        } else {
            siblings
                .iter()
                .position(|id| {
                    by_id
                        .get(id)
                        .is_some_and(|c| c.kind == ChannelType::GuildVoice as u32)
                })
                .unwrap_or(siblings.len())
        };

        siblings.insert(insert_idx, entry.id.clone());
        for (idx, sib_id) in siblings.into_iter().enumerate() {
            if let Some(ch) = by_id.get_mut(&sib_id) {
                ch.position = idx as u32;
            }
        }
    }

    for (cid, (orig_parent, orig_pos)) in &original_state {
        if let Some(ch) = by_id.get(cid)
            && (&ch.parent_id != orig_parent || ch.position != *orig_pos)
        {
            sqlx::query("UPDATE channels SET parent_id = ?, position = ? WHERE id = ?")
                .bind(&ch.parent_id)
                .bind(ch.position)
                .bind(cid)
                .execute(&mut *tx)
                .await?;
            dirty_ids.insert(cid.clone());
        }
    }

    if !lock_copies.is_empty() {
        let owner_id: String = sqlx::query_scalar("SELECT owner_id FROM guilds WHERE id = ?")
            .bind(guild_id)
            .fetch_one(&mut *tx)
            .await?;
        for (cid, pid) in lock_copies {
            let ch_perms =
                permission_service::compute_user_channel_permissions(state, guild_id, &cid, user_id)
                    .await?;
            if !ch_perms.contains(PermissionFlags::MANAGE_ROLES) {
                return Err(AppError::missing_permissions());
            }
            let parent_ows = fetch_channel_overwrites(&mut *tx, &pid).await?;
            let old_ows = fetch_channel_overwrites(&mut *tx, &cid).await?;
            let is_owner_or_admin =
                user_id == owner_id || ch_perms.contains(PermissionFlags::ADMINISTRATOR);
            if !is_owner_or_admin {
                check_overwrite_delta_permissions(ch_perms, &old_ows, &parent_ows)?;
            }
            replace_channel_overwrites(&mut tx, &cid, &parent_ows).await?;
            dirty_ids.insert(cid);
        }
    }

    let mut updated_channels = Vec::with_capacity(dirty_ids.len());
    for cid in dirty_ids {
        if let Some(row) = by_id.remove(&cid) {
            let ows = fetch_channel_overwrites(&mut *tx, &cid).await?;
            updated_channels.push(ChannelResponse::from_row(row, ows));
        }
    }

    tx.commit().await?;

    for ch in updated_channels {
        permission_service::dispatch_channel_with_ctx(
            state,
            guild_id,
            &ch.base.id,
            None,
            events::CHANNEL_UPDATE,
            &ch,
        )
        .await;
    }

    Ok(())
}

async fn fetch_channel_overwrites<'e, E>(
    executor: E,
    channel_id: &str,
) -> Result<Vec<PermissionOverwrite>, AppError>
where
    E: sqlx::Executor<'e, Database = MySql>,
{
    let rows = sqlx::query_as::<_, ChannelOverwriteRow>(
        "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id = ?",
    )
    .bind(channel_id)
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(PermissionOverwrite::from).collect())
}

async fn replace_channel_overwrites(
    tx: &mut sqlx::Transaction<'_, MySql>,
    channel_id: &str,
    overwrites: &[PermissionOverwrite],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM channel_permission_overwrites WHERE channel_id = ?")
        .bind(channel_id)
        .execute(&mut **tx)
        .await?;

    for ow in overwrites {
        sqlx::query(
            "INSERT INTO channel_permission_overwrites (type, channel_id, target_id, allow, deny) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(ow.kind)
        .bind(channel_id)
        .bind(&ow.id)
        .bind(ow.allow)
        .bind(ow.deny)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

async fn validate_and_normalize_overwrites(
    tx: &mut sqlx::Transaction<'_, MySql>,
    guild_id: &str,
    inputs: &[PermissionOverwriteInput],
    actor_perms: PermissionFlags,
    is_owner_or_admin: bool,
    old_overwrites: Option<&[PermissionOverwrite]>,
) -> Result<Vec<PermissionOverwrite>, AppError> {
    if inputs.len() > MAX_CHANNEL_OVERWRITES {
        return Err(AppError::validation(
            "permission_overwrites",
            ValidationCode::InvalidFormat,
            "Too many permission overwrites.",
        ));
    }

    let mut deduped: Vec<PermissionOverwrite> = Vec::with_capacity(inputs.len());
    let mut index_by_id: HashMap<String, usize> = HashMap::with_capacity(inputs.len());

    for input in inputs {
        if input.kind != OverwriteType::Role as u32 && input.kind != OverwriteType::Member as u32 {
            return Err(AppError::validation(
                "permission_overwrites",
                ValidationCode::InvalidFormat,
                "Invalid permission overwrite type.",
            ));
        }
        validate_snowflake("permission_overwrites", &input.id)?;

        let exists: Option<(i32,)> = if input.kind == OverwriteType::Role as u32 {
            sqlx::query_as("SELECT 1 FROM roles WHERE id = ? AND guild_id = ? LIMIT 1")
                .bind(&input.id)
                .bind(guild_id)
                .fetch_optional(&mut **tx)
                .await?
        } else {
            sqlx::query_as("SELECT 1 FROM guild_members WHERE user_id = ? AND guild_id = ? LIMIT 1")
                .bind(&input.id)
                .bind(guild_id)
                .fetch_optional(&mut **tx)
                .await?
        };

        if exists.is_none() {
            return Err(AppError::validation(
                "permission_overwrites",
                ValidationCode::InvalidFormat,
                "Overwrite target does not belong to this community.",
            ));
        }

        let allow = PermissionFlags::from_bits_truncate(input.allow.unwrap_or(0)).bits();
        let deny = PermissionFlags::from_bits_truncate(input.deny.unwrap_or(0)).bits();
        let ow = PermissionOverwrite {
            id: input.id.clone(),
            kind: input.kind,
            allow,
            deny,
        };

        if let Some(&idx) = index_by_id.get(&input.id) {
            deduped[idx] = ow;
        } else {
            index_by_id.insert(input.id.clone(), deduped.len());
            deduped.push(ow);
        }
    }

    if !is_owner_or_admin {
        let empty = Vec::new();
        check_overwrite_delta_permissions(actor_perms, old_overwrites.unwrap_or(&empty), &deduped)?;
    }

    Ok(deduped)
}

fn check_overwrite_delta_permissions(
    actor_perms: PermissionFlags,
    old_overwrites: &[PermissionOverwrite],
    new_overwrites: &[PermissionOverwrite],
) -> Result<(), AppError> {
    let old_by_id: HashMap<&str, &PermissionOverwrite> =
        old_overwrites.iter().map(|o| (o.id.as_str(), o)).collect();
    let new_by_id: HashMap<&str, &PermissionOverwrite> =
        new_overwrites.iter().map(|o| (o.id.as_str(), o)).collect();

    for ow in new_overwrites {
        if (ow.allow & !actor_perms.bits()) != 0 {
            return Err(AppError::missing_permissions());
        }
        if let Some(prev) = old_by_id.get(ow.id.as_str()) {
            let cleared_denies = prev.deny & !ow.deny;
            if (cleared_denies & !actor_perms.bits()) != 0 {
                return Err(AppError::missing_permissions());
            }
        }
    }

    for prev in old_overwrites {
        if !new_by_id.contains_key(prev.id.as_str()) && (prev.deny & !actor_perms.bits()) != 0 {
            return Err(AppError::missing_permissions());
        }
    }

    Ok(())
}

fn sorted_overwrite_key(ows: &[PermissionOverwrite]) -> Vec<(u32, String, u64, u64)> {
    let mut keys: Vec<(u32, String, u64, u64)> = ows
        .iter()
        .map(|o| (o.kind, o.id.clone(), o.allow, o.deny))
        .collect();
    keys.sort();
    keys
}

async fn propagate_category_overwrites(
    tx: &mut sqlx::Transaction<'_, MySql>,
    guild_id: &str,
    category_id: &str,
    old_category_overwrites: &[PermissionOverwrite],
    new_category_overwrites: &[PermissionOverwrite],
) -> Result<Vec<ChannelResponse>, AppError> {
    let old_key = sorted_overwrite_key(old_category_overwrites);
    let children = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE guild_id = ? AND parent_id = ?",
        ChannelRow::SELECT_COLS
    ))
    .bind(guild_id)
    .bind(category_id)
    .fetch_all(&mut **tx)
    .await?;

    let mut synced = Vec::new();
    for child in children {
        let child_ows = fetch_channel_overwrites(&mut **tx, &child.id).await?;
        if sorted_overwrite_key(&child_ows) == old_key {
            replace_channel_overwrites(tx, &child.id, new_category_overwrites).await?;
            synced.push(ChannelResponse::from_row(
                child,
                new_category_overwrites.to_vec(),
            ));
        }
    }
    Ok(synced)
}

async fn dispatch_channel_update_with_visibility(
    state: &AppState,
    guild_id: &str,
    channel_id: &str,
    old_sessions: Vec<Arc<GatewaySession>>,
    updated: &ChannelResponse,
) {
    let new_sessions =
        permission_service::filter_sessions_for_channel_with_ctx(state, guild_id, channel_id, None)
            .await;

    let old_ids: HashSet<String> = old_sessions.iter().map(|s| s.id.clone()).collect();
    let new_ids: HashSet<String> = new_sessions.iter().map(|s| s.id.clone()).collect();

    let update_targets: Vec<Arc<GatewaySession>> = new_sessions
        .iter()
        .filter(|s| old_ids.contains(&s.id))
        .cloned()
        .collect();
    let create_targets: Vec<Arc<GatewaySession>> = new_sessions
        .into_iter()
        .filter(|s| !old_ids.contains(&s.id))
        .collect();
    let delete_targets: Vec<Arc<GatewaySession>> = old_sessions
        .into_iter()
        .filter(|s| !new_ids.contains(&s.id))
        .collect();

    state.dispatch_to_sessions(&update_targets, events::CHANNEL_UPDATE, updated);
    state.dispatch_to_sessions(&create_targets, events::CHANNEL_CREATE, updated);
    state.dispatch_to_sessions(&delete_targets, events::CHANNEL_DELETE, updated);
}

fn is_supported_guild_channel_type(kind: u32) -> bool {
    kind == ChannelType::GuildText as u32
        || kind == ChannelType::GuildVoice as u32
        || kind == ChannelType::GuildCategory as u32
        || kind == ChannelType::GuildAnnouncement as u32
        || kind == ChannelType::GuildLink as u32
}

fn normalize_channel_name(raw: &str, kind: u32) -> Result<String, AppError> {
    let mut collapsed = String::with_capacity(raw.len());
    let mut prev_space = true;
    for c in raw.chars() {
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
        trimmed
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
            .collect::<String>()
    } else {
        trimmed.to_string()
    };

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

fn validate_topic_opt(raw: Option<&str>) -> Result<Option<String>, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MAX_CHANNEL_TOPIC_LENGTH {
        return Err(AppError::validation(
            "topic",
            ValidationCode::ContentExceedsMaxLength,
            "Channel topic must not exceed 1024 characters.",
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
