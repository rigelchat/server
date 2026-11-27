use std::collections::{BTreeMap, HashMap, HashSet};

use sqlx::{Executor, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::{
    AUTO_ARCHIVE_DURATIONS, ChannelType, DEFAULT_FORUM_TAG_SETTING, ForumLayout, ForumSortOrder,
    VALID_FORUM_TAG_SETTINGS,
};
use crate::constants::flags::{ChannelFlags, PermissionFlags};
use crate::constants::gateway::events;
use crate::constants::limits::{
    MAX_CHANNEL_RATE_LIMIT_PER_USER, MAX_FORUM_POST_DATA_IDS, MAX_FORUM_TAG_NAME_LENGTH,
    MAX_FORUM_TAGS,
};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::{ChannelResponse, ChannelRow, ForumTag, ForumTagRow};
use crate::models::guild_member::{GuildMemberResponse, GuildMemberUserRow};
use crate::models::message::{Message, MessageRow};
use crate::models::rest::channels::{DefaultReactionInput, ForumTagInput};
use crate::models::rest::forums::{
    CreateForumTagRequest, ForumPostDataItem, ForumPostDataResponse, GetForumPostDataRequest,
    UpdateForumTagRequest,
};
use crate::services::{permission_service, queries};
use crate::utils::snowflake::{DISCORD_SNOWFLAKE, validate_snowflake};
use crate::utils::validation::{reject_control_chars, validate_unicode_emoji};

pub async fn fetch_channel_forum_tags<'e, E>(
    executor: E,
    channel_id: &str,
) -> Result<Vec<ForumTag>, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    let rows = sqlx::query_as::<_, ForumTagRow>(
        "SELECT id, channel_id, name, moderated, emoji_id, emoji_name FROM forum_tags WHERE channel_id = ? ORDER BY position ASC, CAST(id AS UNSIGNED) ASC",
    )
    .bind(channel_id)
    .fetch_all(executor)
    .await?;

    Ok(rows.into_iter().map(ForumTag::from).collect())
}

pub async fn fetch_guild_forum_tags<'e, E>(
    executor: E,
    guild_id: &str,
) -> Result<HashMap<String, Vec<ForumTag>>, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    let rows = sqlx::query_as::<_, ForumTagRow>(
        r#"
        SELECT t.id, t.channel_id, t.name, t.moderated, t.emoji_id, t.emoji_name
        FROM forum_tags t
        INNER JOIN channels c ON c.id = t.channel_id
        WHERE c.guild_id = ?
        ORDER BY t.position ASC, CAST(t.id AS UNSIGNED) ASC
        "#,
    )
    .bind(guild_id)
    .fetch_all(executor)
    .await?;

    let mut by_channel: HashMap<String, Vec<ForumTag>> = HashMap::new();
    for row in rows {
        by_channel
            .entry(row.channel_id.clone())
            .or_default()
            .push(ForumTag::from(row));
    }
    Ok(by_channel)
}

pub(super) async fn replace_channel_forum_tags(
    tx: &mut sqlx::Transaction<'_, MySql>,
    channel_id: &str,
    existing_ids: &HashSet<String>,
    tags: &[ForumTag],
) -> Result<(), AppError> {
    let kept_ids: Vec<&str> = tags
        .iter()
        .filter(|t| existing_ids.contains(&t.id))
        .map(|t| t.id.as_str())
        .collect();

    if kept_ids.is_empty() {
        sqlx::query("DELETE FROM forum_tags WHERE channel_id = ?")
            .bind(channel_id)
            .execute(&mut **tx)
            .await?;
    } else {
        let mut dqb = QueryBuilder::<MySql>::new("DELETE FROM forum_tags WHERE channel_id = ");
        dqb.push_bind(channel_id);
        dqb.push(" AND id NOT IN (");
        let mut sep = dqb.separated(", ");
        for id in &kept_ids {
            sep.push_bind(*id);
        }
        dqb.push(")");
        dqb.build().execute(&mut **tx).await?;

        sqlx::query("UPDATE forum_tags SET name = CONCAT('__tmp_', id) WHERE channel_id = ?")
            .bind(channel_id)
            .execute(&mut **tx)
            .await?;
    }

    for (idx, tag) in tags.iter().enumerate() {
        let pos = idx as u32;
        if existing_ids.contains(&tag.id) {
            sqlx::query(
                "UPDATE forum_tags SET position = ?, name = ?, moderated = ?, emoji_id = ?, emoji_name = ? WHERE id = ? AND channel_id = ?",
            )
            .bind(pos)
            .bind(&tag.name)
            .bind(tag.moderated)
            .bind(&tag.emoji_id)
            .bind(&tag.emoji_name)
            .bind(&tag.id)
            .bind(channel_id)
            .execute(&mut **tx)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO forum_tags (id, channel_id, position, name, moderated, emoji_id, emoji_name) VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&tag.id)
            .bind(channel_id)
            .bind(pos)
            .bind(&tag.name)
            .bind(tag.moderated)
            .bind(&tag.emoji_id)
            .bind(&tag.emoji_name)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}

pub(super) fn validate_forum_flags(raw: u32, kind: u32) -> Result<u32, AppError> {
    if kind == ChannelType::GuildForum as u32
        && (raw & ChannelFlags::HIDE_MEDIA_DOWNLOAD_OPTIONS.bits()) != 0
    {
        return Err(AppError::hide_media_download_option_media_only());
    }
    let allowed = if kind == ChannelType::GuildMedia as u32 {
        ChannelFlags::MEDIA_ALLOWED.bits()
    } else {
        ChannelFlags::FORUM_ALLOWED.bits()
    };
    if (raw & !allowed) != 0 {
        return Err(AppError::validation(
            "flags",
            ValidationCode::InvalidFormat,
            "Invalid channel flags.",
        ));
    }
    Ok(raw)
}

pub(super) fn validate_default_sort_order(raw: Option<u8>) -> Result<Option<u8>, AppError> {
    if let Some(val) = raw
        && !ForumSortOrder::is_valid(val)
    {
        return Err(AppError::validation(
            "default_sort_order",
            ValidationCode::ValueMustBeIntegerInRange,
            "default_sort_order must be 0 or 1.",
        ));
    }
    Ok(raw)
}

pub(super) fn validate_default_forum_layout(raw: Option<u8>) -> Result<u8, AppError> {
    let val = raw.unwrap_or(ForumLayout::Default as u8);
    if !ForumLayout::is_valid(val) {
        return Err(AppError::validation(
            "default_forum_layout",
            ValidationCode::ValueMustBeIntegerInRange,
            "default_forum_layout must be between 0 and 2.",
        ));
    }
    Ok(val)
}

pub(super) fn validate_default_tag_setting(raw: Option<&str>) -> Result<String, AppError> {
    let Some(trimmed) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(DEFAULT_FORUM_TAG_SETTING.to_string());
    };
    if !VALID_FORUM_TAG_SETTINGS.contains(&trimmed) {
        return Err(AppError::validation(
            "default_tag_setting",
            ValidationCode::InvalidFormat,
            "default_tag_setting must be 'match_some' or 'match_all'.",
        ));
    }
    Ok(trimmed.to_string())
}

pub(super) fn validate_default_auto_archive_duration(
    raw: Option<u32>,
) -> Result<Option<u32>, AppError> {
    if let Some(val) = raw
        && !AUTO_ARCHIVE_DURATIONS.contains(&val)
    {
        return Err(AppError::validation(
            "default_auto_archive_duration",
            ValidationCode::ValueMustBeIntegerInRange,
            "default_auto_archive_duration must be 60, 1440, 4320, or 10080.",
        ));
    }
    Ok(raw)
}

pub(super) fn validate_default_thread_rate_limit(raw: Option<u32>) -> Result<u32, AppError> {
    let val = raw.unwrap_or(0);
    if val > MAX_CHANNEL_RATE_LIMIT_PER_USER {
        return Err(AppError::validation(
            "default_thread_rate_limit_per_user",
            ValidationCode::ValueMustBeIntegerInRange,
            "Slowmode must be between 0 and 21600 seconds.",
        ));
    }
    Ok(val)
}

pub(super) fn validate_tag_emoji(
    field_prefix: &str,
    emoji_id: Option<&str>,
    emoji_name: Option<&str>,
) -> Result<(Option<String>, Option<String>), AppError> {
    let id = emoji_id.map(str::trim).filter(|s| !s.is_empty());
    let name = emoji_name.map(str::trim).filter(|s| !s.is_empty());

    if id.is_some() && name.is_some() {
        return Err(AppError::validation(
            field_prefix,
            ValidationCode::InvalidFormat,
            "Cannot set both emoji_id and emoji_name.",
        ));
    }

    if let Some(eid) = id {
        validate_snowflake(field_prefix, eid)?;
        return Err(AppError::unknown_emoji());
    }

    if let Some(ename) = name {
        let validated = validate_unicode_emoji(field_prefix, ename)?;
        return Ok((None, Some(validated)));
    }

    Ok((None, None))
}

pub(super) fn validate_default_reaction_input(
    input: Option<DefaultReactionInput>,
) -> Result<(Option<String>, Option<String>), AppError> {
    let Some(reaction) = input else {
        return Ok((None, None));
    };
    let eid = reaction
        .emoji_id
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let ename = reaction
        .emoji_name
        .flatten()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    if eid.is_none() && ename.is_none() {
        return Err(AppError::validation(
            "default_reaction_emoji",
            ValidationCode::InvalidFormat,
            "Either emoji_id or emoji_name must be provided.",
        ));
    }

    validate_tag_emoji("default_reaction_emoji", eid.as_deref(), ename.as_deref())
}

fn validate_tag_name(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_FORUM_TAG_NAME_LENGTH {
        return Err(AppError::validation(
            "name",
            ValidationCode::StringLengthInvalid,
            "Tag name must be between 1 and 50 characters.",
        ));
    }
    reject_control_chars("name", trimmed)?;
    Ok(trimmed.to_string())
}

pub(super) fn validate_and_normalize_tags_list(
    inputs: &[ForumTagInput],
    existing_ids: &HashSet<String>,
) -> Result<Vec<ForumTag>, AppError> {
    if inputs.len() > MAX_FORUM_TAGS {
        return Err(AppError::max_forum_tags());
    }

    let mut out = Vec::with_capacity(inputs.len());
    let mut seen_names = HashSet::with_capacity(inputs.len());
    let mut seen_ids = HashSet::with_capacity(inputs.len());

    for input in inputs {
        let name = validate_tag_name(&input.name)?;
        if !seen_names.insert(name.clone()) {
            return Err(AppError::forum_tag_names_must_be_unique());
        }

        let id = match input.id.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(existing_id) => {
                if !existing_ids.contains(existing_id) || !seen_ids.insert(existing_id.to_string())
                {
                    return Err(AppError::unknown_forum_tag());
                }
                existing_id.to_string()
            }
            None => DISCORD_SNOWFLAKE.generate(None).to_string(),
        };

        let (emoji_id, emoji_name) = validate_tag_emoji(
            "available_tags",
            input.emoji_id.as_deref(),
            input.emoji_name.as_deref(),
        )?;

        out.push(ForumTag {
            id,
            name,
            moderated: input.moderated.unwrap_or(false),
            emoji_id,
            emoji_name,
        });
    }

    Ok(out)
}

pub(super) fn ensure_unmoderated_tag_if_required(
    flags: u32,
    tags: &[ForumTag],
) -> Result<(), AppError> {
    if (flags & ChannelFlags::REQUIRE_TAG.bits()) != 0 && !tags.iter().any(|t| !t.moderated) {
        return Err(AppError::no_tags_available_to_non_moderators());
    }
    Ok(())
}

async fn lock_forum_channel_for_tag_edit(
    tx: &mut sqlx::Transaction<'_, MySql>,
    channel_id: &str,
    user_id: &str,
) -> Result<ChannelRow, AppError> {
    let row = sqlx::query_as::<_, ChannelRow>(&format!(
        "SELECT {} FROM channels WHERE id = ? FOR UPDATE",
        ChannelRow::SELECT_COLS
    ))
    .bind(channel_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(AppError::not_found)?;

    let perms = permission_service::compute_user_channel_permissions_conn(
        tx,
        &row.guild_id,
        channel_id,
        user_id,
    )
    .await?;
    if !perms.contains(PermissionFlags::VIEW_CHANNEL | PermissionFlags::MANAGE_CHANNELS) {
        return Err(AppError::missing_permissions());
    }

    if !ChannelType::is_forum_like(row.kind) {
        return Err(AppError::invalid_channel_type());
    }

    Ok(row)
}

pub async fn create_forum_tag(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    payload: CreateForumTagRequest,
) -> Result<ChannelResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }

    let name = validate_tag_name(&payload.name)?;
    let moderated = payload.moderated.unwrap_or(false);
    let (emoji_id, emoji_name) = validate_tag_emoji(
        "emoji_name",
        payload.emoji_id.flatten().as_deref(),
        payload.emoji_name.flatten().as_deref(),
    )?;

    let mut tx = state.db.begin().await?;
    let row = lock_forum_channel_for_tag_edit(&mut tx, channel_id, user_id).await?;
    let mut tags = fetch_channel_forum_tags(&mut *tx, channel_id).await?;

    if tags.len() >= MAX_FORUM_TAGS {
        return Err(AppError::max_forum_tags());
    }
    if tags.iter().any(|t| t.name == name) {
        return Err(AppError::forum_tag_names_must_be_unique());
    }

    let max_pos: Option<u32> =
        sqlx::query_scalar("SELECT MAX(position) FROM forum_tags WHERE channel_id = ?")
            .bind(channel_id)
            .fetch_one(&mut *tx)
            .await?;
    let next_pos = max_pos.map_or(0, |p| p.saturating_add(1));

    let tag = ForumTag {
        id: DISCORD_SNOWFLAKE.generate(None).to_string(),
        name,
        moderated,
        emoji_id,
        emoji_name,
    };

    let mut preview_tags = tags.clone();
    preview_tags.push(tag.clone());
    ensure_unmoderated_tag_if_required(row.flags, &preview_tags)?;

    sqlx::query(
        "INSERT INTO forum_tags (id, channel_id, position, name, moderated, emoji_id, emoji_name) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&tag.id)
    .bind(channel_id)
    .bind(next_pos)
    .bind(&tag.name)
    .bind(tag.moderated)
    .bind(&tag.emoji_id)
    .bind(&tag.emoji_name)
    .execute(&mut *tx)
    .await?;

    tags.push(tag);
    let overwrites = queries::fetch_channel_overwrites(&mut *tx, channel_id).await?;
    tx.commit().await?;

    let guild_id = row.guild_id.clone();
    let updated = ChannelResponse::from_row(row, overwrites, tags);
    permission_service::dispatch_channel_with_ctx(
        state,
        &guild_id,
        channel_id,
        None,
        events::CHANNEL_UPDATE,
        &updated,
    )
    .await;

    Ok(updated)
}

pub async fn update_forum_tag(
    state: &AppState,
    channel_id: &str,
    tag_id: &str,
    user_id: &str,
    can_use_threads: bool,
    payload: UpdateForumTagRequest,
) -> Result<ChannelResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }
    if validate_snowflake("tag_id", tag_id).is_err() {
        return Err(AppError::unknown_forum_tag());
    }

    let name = validate_tag_name(&payload.name)?;
    let moderated = payload.moderated.unwrap_or(false);
    let (emoji_id, emoji_name) = validate_tag_emoji(
        "emoji_name",
        payload.emoji_id.flatten().as_deref(),
        payload.emoji_name.flatten().as_deref(),
    )?;

    let mut tx = state.db.begin().await?;
    let row = lock_forum_channel_for_tag_edit(&mut tx, channel_id, user_id).await?;
    let mut tags = fetch_channel_forum_tags(&mut *tx, channel_id).await?;

    let Some(idx) = tags.iter().position(|t| t.id == tag_id) else {
        return Err(AppError::unknown_forum_tag());
    };
    if tags.iter().any(|t| t.id != tag_id && t.name == name) {
        return Err(AppError::forum_tag_names_must_be_unique());
    }

    tags[idx].name = name.clone();
    tags[idx].moderated = moderated;
    tags[idx].emoji_id = emoji_id.clone();
    tags[idx].emoji_name = emoji_name.clone();

    ensure_unmoderated_tag_if_required(row.flags, &tags)?;

    sqlx::query(
        "UPDATE forum_tags SET name = ?, moderated = ?, emoji_id = ?, emoji_name = ? WHERE id = ? AND channel_id = ?",
    )
    .bind(&name)
    .bind(moderated)
    .bind(&emoji_id)
    .bind(&emoji_name)
    .bind(tag_id)
    .bind(channel_id)
    .execute(&mut *tx)
    .await?;

    let overwrites = queries::fetch_channel_overwrites(&mut *tx, channel_id).await?;
    tx.commit().await?;

    let guild_id = row.guild_id.clone();
    let updated = ChannelResponse::from_row(row, overwrites, tags);
    permission_service::dispatch_channel_with_ctx(
        state,
        &guild_id,
        channel_id,
        None,
        events::CHANNEL_UPDATE,
        &updated,
    )
    .await;

    Ok(updated)
}

pub async fn delete_forum_tag(
    state: &AppState,
    channel_id: &str,
    tag_id: &str,
    user_id: &str,
    can_use_threads: bool,
) -> Result<ChannelResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }
    if validate_snowflake("tag_id", tag_id).is_err() {
        return Err(AppError::unknown_forum_tag());
    }

    let mut tx = state.db.begin().await?;
    let row = lock_forum_channel_for_tag_edit(&mut tx, channel_id, user_id).await?;
    let mut tags = fetch_channel_forum_tags(&mut *tx, channel_id).await?;

    let Some(idx) = tags.iter().position(|t| t.id == tag_id) else {
        return Err(AppError::unknown_forum_tag());
    };
    tags.remove(idx);

    ensure_unmoderated_tag_if_required(row.flags, &tags)?;

    sqlx::query("DELETE FROM forum_tags WHERE id = ? AND channel_id = ?")
        .bind(tag_id)
        .bind(channel_id)
        .execute(&mut *tx)
        .await?;

    let overwrites = queries::fetch_channel_overwrites(&mut *tx, channel_id).await?;
    tx.commit().await?;

    let guild_id = row.guild_id.clone();
    let updated = ChannelResponse::from_row(row, overwrites, tags);
    permission_service::dispatch_channel_with_ctx(
        state,
        &guild_id,
        channel_id,
        None,
        events::CHANNEL_UPDATE,
        &updated,
    )
    .await;

    Ok(updated)
}

pub async fn get_forum_post_data(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    payload: GetForumPostDataRequest,
) -> Result<ForumPostDataResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }

    if payload.thread_ids.is_empty() || payload.thread_ids.len() > MAX_FORUM_POST_DATA_IDS {
        return Err(AppError::validation(
            "thread_ids",
            ValidationCode::InvalidFormat,
            "thread_ids must contain between 1 and 100 snowflakes.",
        ));
    }

    let mut unique_ids = Vec::with_capacity(payload.thread_ids.len());
    let mut seen = HashSet::with_capacity(payload.thread_ids.len());
    for tid in &payload.thread_ids {
        validate_snowflake("thread_ids", tid)?;
        if seen.insert(tid.clone()) {
            unique_ids.push(tid.clone());
        }
    }

    let meta: Option<(String, u32)> =
        sqlx::query_as("SELECT guild_id, type FROM channels WHERE id = ?")
            .bind(channel_id)
            .fetch_optional(&state.db)
            .await?;
    let Some((guild_id, kind)) = meta else {
        return Err(AppError::not_found());
    };

    permission_service::require_channel_permission(
        state,
        &guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::READ_MESSAGE_HISTORY,
    )
    .await?;

    if !ChannelType::is_forum_like(kind) {
        return Err(AppError::invalid_channel_type());
    }

    let mut tqb = QueryBuilder::<MySql>::new(
        "SELECT channel_id, owner_id FROM channel_threads WHERE parent_id = ",
    );
    tqb.push_bind(channel_id);
    tqb.push(" AND channel_id IN (");
    let mut sep = tqb.separated(", ");
    for tid in &unique_ids {
        sep.push_bind(tid);
    }
    tqb.push(")");

    let thread_rows: Vec<(String, String)> = tqb.build_query_as().fetch_all(&state.db).await?;
    let post_owners: HashMap<String, String> = thread_rows.into_iter().collect();

    let mut owners_by_user: HashMap<String, GuildMemberResponse> = HashMap::new();
    let mut messages_by_id: HashMap<String, Message> = HashMap::new();

    if !post_owners.is_empty() {
        let owner_ids: Vec<String> = post_owners
            .values()
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        let mut mqb = QueryBuilder::<MySql>::new(
            r#"
            SELECT
                gm.user_id, gm.joined_at, gm.nick,
                u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
            FROM guild_members gm
            INNER JOIN users u ON u.id = gm.user_id
            WHERE gm.guild_id = 
            "#,
        );
        mqb.push_bind(&guild_id);
        mqb.push(" AND gm.user_id IN (");
        let mut sep = mqb.separated(", ");
        for oid in &owner_ids {
            sep.push_bind(oid);
        }
        mqb.push(")");
        let member_rows = mqb
            .build_query_as::<GuildMemberUserRow>()
            .fetch_all(&state.db)
            .await?;

        let mut roles_by_user: HashMap<String, Vec<String>> = HashMap::new();
        if !member_rows.is_empty() {
            let mut rqb = QueryBuilder::<MySql>::new(
                "SELECT user_id, role_id FROM guild_member_roles WHERE guild_id = ",
            );
            rqb.push_bind(&guild_id);
            rqb.push(" AND role_id != ");
            rqb.push_bind(&guild_id);
            rqb.push(" AND user_id IN (");
            let mut sep = rqb.separated(", ");
            for oid in &owner_ids {
                sep.push_bind(oid);
            }
            rqb.push(")");
            let role_rows: Vec<(String, String)> =
                rqb.build_query_as().fetch_all(&state.db).await?;
            for (uid, rid) in role_rows {
                roles_by_user.entry(uid).or_default().push(rid);
            }
        }

        for mr in member_rows {
            let roles = roles_by_user.remove(&mr.user_id).unwrap_or_default();
            let uid = mr.user_id.clone();
            owners_by_user.insert(uid, mr.into_member_response(roles));
        }

        let mut msg_qb =
            QueryBuilder::<MySql>::new(format!("{} WHERE m.id IN (", MessageRow::SELECT_FROM));
        let mut sep = msg_qb.separated(", ");
        for pid in post_owners.keys() {
            sep.push_bind(pid);
        }
        msg_qb.push(") AND m.channel_id = m.id");
        let msg_rows = msg_qb
            .build_query_as::<MessageRow>()
            .fetch_all(&state.db)
            .await?;
        let hydrated = crate::services::message_service::hydrate_messages(
            state,
            msg_rows,
            user_id,
            can_use_threads,
        )
        .await?;
        for m in hydrated {
            messages_by_id.insert(m.id.clone(), m);
        }
    }

    let mut threads = BTreeMap::new();
    for tid in payload.thread_ids {
        if let Some(owner_id) = post_owners.get(&tid) {
            threads.insert(
                tid.clone(),
                ForumPostDataItem {
                    owner: owners_by_user.get(owner_id).cloned(),
                    first_message: messages_by_id.get(&tid).cloned(),
                },
            );
        } else {
            threads.insert(
                tid,
                ForumPostDataItem {
                    owner: None,
                    first_message: None,
                },
            );
        }
    }

    Ok(ForumPostDataResponse { threads })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forum_flags_and_defaults_validate_correctly() {
        assert!(
            validate_forum_flags(
                ChannelFlags::REQUIRE_TAG.bits(),
                ChannelType::GuildForum as u32
            )
            .is_ok()
        );
        assert!(
            validate_forum_flags(
                ChannelFlags::HIDE_MEDIA_DOWNLOAD_OPTIONS.bits(),
                ChannelType::GuildForum as u32
            )
            .is_err()
        );
        assert!(
            validate_forum_flags(
                ChannelFlags::HIDE_MEDIA_DOWNLOAD_OPTIONS.bits(),
                ChannelType::GuildMedia as u32
            )
            .is_ok()
        );
        assert!(
            validate_forum_flags(ChannelFlags::PINNED.bits(), ChannelType::GuildForum as u32)
                .is_err()
        );

        assert!(validate_default_sort_order(Some(0)).is_ok());
        assert!(validate_default_sort_order(Some(2)).is_err());
        assert!(validate_default_forum_layout(Some(2)).is_ok());
        assert!(validate_default_forum_layout(Some(3)).is_err());
        assert_eq!(validate_default_tag_setting(None).unwrap(), "match_some");
        assert!(validate_default_tag_setting(Some("match_all")).is_ok());
        assert!(validate_default_tag_setting(Some("invalid")).is_err());
    }

    #[test]
    fn require_tag_checks_unmoderated_availability() {
        let mod_only = vec![ForumTag {
            id: "1".into(),
            name: "Mod".into(),
            moderated: true,
            emoji_id: None,
            emoji_name: None,
        }];
        assert!(
            ensure_unmoderated_tag_if_required(ChannelFlags::REQUIRE_TAG.bits(), &mod_only)
                .is_err()
        );
        assert!(ensure_unmoderated_tag_if_required(0, &mod_only).is_ok());

        let with_open = vec![
            mod_only[0].clone(),
            ForumTag {
                id: "2".into(),
                name: "Help".into(),
                moderated: false,
                emoji_id: None,
                emoji_name: None,
            },
        ];
        assert!(
            ensure_unmoderated_tag_if_required(ChannelFlags::REQUIRE_TAG.bits(), &with_open)
                .is_ok()
        );
    }
}
