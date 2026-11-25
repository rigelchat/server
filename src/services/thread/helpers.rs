use std::collections::{HashMap, HashSet};

use sqlx::{Executor, MySql, QueryBuilder};

use crate::constants::channel::{AUTO_ARCHIVE_DURATIONS, ChannelType};
use crate::constants::flags::{ChannelFlags, PermissionFlags};
use crate::constants::limits::{
    MAX_ACTIVE_THREADS_PER_GUILD, MAX_APPLIED_TAGS, MAX_CHANNEL_NAME_LENGTH,
    MAX_CHANNEL_RATE_LIMIT_PER_USER, MAX_THREAD_MEMBER_IDS_PREVIEW,
};
use crate::error::{AppError, ValidationCode};
use crate::models::thread::{ThreadDbRow, ThreadMember, ThreadMemberDbRow, ThreadResponse};
use crate::services::permission_service;
use crate::utils::snowflake::validate_snowflake;
use crate::utils::validation::reject_control_chars;

pub(super) fn validate_thread_name(raw: &str) -> Result<String, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > MAX_CHANNEL_NAME_LENGTH {
        return Err(AppError::validation(
            "name",
            ValidationCode::StringLengthInvalid,
            "Thread name must be between 1 and 100 characters.",
        ));
    }
    reject_control_chars("name", trimmed)?;
    Ok(trimmed.to_string())
}

pub(super) fn validate_auto_archive_duration_val(val: u32) -> Result<u32, AppError> {
    if !AUTO_ARCHIVE_DURATIONS.contains(&val) {
        return Err(AppError::validation(
            "auto_archive_duration",
            ValidationCode::ValueMustBeIntegerInRange,
            "auto_archive_duration must be 60, 1440, 4320, or 10080.",
        ));
    }
    Ok(val)
}

pub(super) fn validate_thread_rate_limit(val: u32) -> Result<u32, AppError> {
    if val > MAX_CHANNEL_RATE_LIMIT_PER_USER {
        return Err(AppError::validation(
            "rate_limit_per_user",
            ValidationCode::ValueMustBeIntegerInRange,
            "Slowmode must be between 0 and 21600 seconds.",
        ));
    }
    Ok(val)
}

pub(super) async fn fetch_thread_row<'e, E>(
    executor: E,
    thread_id: &str,
) -> Result<Option<ThreadDbRow>, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    let sql = format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE c.id = ?",
        ThreadDbRow::SELECT_COLS
    );
    Ok(sqlx::query_as::<_, ThreadDbRow>(&sql)
        .bind(thread_id)
        .fetch_optional(executor)
        .await?)
}

pub(super) async fn is_thread_member<'e, E>(
    executor: E,
    thread_id: &str,
    user_id: &str,
) -> Result<bool, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    let exists: Option<(i32,)> = sqlx::query_as(
        "SELECT 1 FROM thread_members WHERE thread_id = ? AND user_id = ? LIMIT 1",
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?;
    Ok(exists.is_some())
}

pub(super) async fn fetch_thread_member_self<'e, E>(
    executor: E,
    thread_id: &str,
    user_id: &str,
) -> Result<Option<ThreadMemberDbRow>, AppError>
where
    E: Executor<'e, Database = MySql>,
{
    Ok(sqlx::query_as::<_, ThreadMemberDbRow>(
        "SELECT thread_id, user_id, join_timestamp, flags, muted, mute_config FROM thread_members WHERE thread_id = ? AND user_id = ?",
    )
    .bind(thread_id)
    .bind(user_id)
    .fetch_optional(executor)
    .await?)
}

pub(super) async fn check_thread_view_access(
    conn: &mut sqlx::MySqlConnection,
    row: &ThreadDbRow,
    user_id: &str,
) -> Result<(PermissionFlags, PermissionFlags, bool), AppError> {
    let (parent_perms, thread_perms) = permission_service::compute_user_thread_permissions_conn(
        conn,
        &row.guild_id,
        &row.parent_id,
        user_id,
    )
    .await
    .map_err(|err| match err {
        AppError::Coded {
            code: crate::error::ApiErrorCode::MissingPermissions,
            ..
        } => AppError::missing_access(),
        other => other,
    })?;

    if !parent_perms.contains(PermissionFlags::VIEW_CHANNEL) {
        return Err(AppError::missing_access());
    }

    let is_mod = parent_perms.contains(PermissionFlags::MANAGE_THREADS);
    let is_member = is_thread_member(&mut *conn, &row.id, user_id).await?;
    if row.kind == ChannelType::PrivateThread as u32 && !is_mod && !is_member {
        return Err(AppError::missing_access());
    }

    Ok((parent_perms, thread_perms, is_member))
}

pub(super) async fn ensure_active_thread_capacity(
    tx: &mut sqlx::Transaction<'_, MySql>,
    guild_id: &str,
) -> Result<(), AppError> {
    let active_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM channel_threads WHERE guild_id = ? AND archived = FALSE",
    )
    .bind(guild_id)
    .fetch_one(&mut **tx)
    .await?;
    if active_count >= MAX_ACTIVE_THREADS_PER_GUILD {
        return Err(AppError::max_active_threads());
    }
    Ok(())
}

pub(super) async fn validate_and_prepare_applied_tags(
    tx: &mut sqlx::Transaction<'_, MySql>,
    parent_id: &str,
    parent_flags: u32,
    requested: Option<&[String]>,
    existing_tags: Option<&[String]>,
    is_moderator: bool,
) -> Result<Vec<String>, AppError> {
    let raw_ids = requested.unwrap_or(&[]);
    if raw_ids.len() > MAX_APPLIED_TAGS {
        return Err(AppError::validation(
            "applied_tags",
            ValidationCode::InvalidFormat,
            "At most 5 tags can be applied to a post.",
        ));
    }

    let all_parent_tags: Vec<(String, bool)> = sqlx::query_as(
        "SELECT id, moderated FROM forum_tags WHERE channel_id = ?",
    )
    .bind(parent_id)
    .fetch_all(&mut **tx)
    .await?;

    let mod_by_id: HashMap<String, bool> = all_parent_tags.iter().cloned().collect();
    let has_unmoderated_option = all_parent_tags.iter().any(|(_, m)| !*m);

    let mut deduped = Vec::with_capacity(raw_ids.len());
    let mut seen = HashSet::with_capacity(raw_ids.len());
    for tid in raw_ids {
        let trimmed = tid.trim();
        if validate_snowflake("applied_tags", trimmed).is_err() {
            return Err(AppError::unknown_forum_tag());
        }
        if !mod_by_id.contains_key(trimmed) {
            return Err(AppError::unknown_forum_tag());
        }
        if seen.insert(trimmed.to_string()) {
            deduped.push(trimmed.to_string());
        }
    }

    if !is_moderator {
        let old_set: HashSet<&str> = existing_tags
            .map(|s| s.iter().map(String::as_str).collect())
            .unwrap_or_default();
        let new_set: HashSet<&str> = deduped.iter().map(String::as_str).collect();

        for (tid, is_mod_tag) in &mod_by_id {
            if *is_mod_tag && (old_set.contains(tid.as_str()) != new_set.contains(tid.as_str())) {
                return Err(AppError::missing_permissions());
            }
        }
    }

    if (parent_flags & ChannelFlags::REQUIRE_TAG.bits()) != 0 && deduped.is_empty() {
        if !is_moderator && !has_unmoderated_option {
            return Err(AppError::no_tags_available_to_non_moderators());
        }
        return Err(AppError::forum_tag_required());
    }

    Ok(deduped)
}

pub(super) async fn sync_thread_applied_tags(
    tx: &mut sqlx::Transaction<'_, MySql>,
    thread_id: &str,
    tag_ids: &[String],
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM thread_applied_tags WHERE thread_id = ?")
        .bind(thread_id)
        .execute(&mut **tx)
        .await?;

    for (idx, tag_id) in tag_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO thread_applied_tags (thread_id, tag_id, position) VALUES (?, ?, ?)",
        )
        .bind(thread_id)
        .bind(tag_id)
        .bind(idx as u32)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

pub(super) async fn add_thread_member_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    thread_id: &str,
    user_id: &str,
    flags: u32,
    now_ms: u64,
) -> Result<(bool, ThreadMemberDbRow, u32), AppError> {
    let existing = fetch_thread_member_self(&mut **tx, thread_id, user_id).await?;
    if let Some(row) = existing {
        let count: u32 = sqlx::query_scalar(
            "SELECT member_count FROM channel_threads WHERE channel_id = ?",
        )
        .bind(thread_id)
        .fetch_one(&mut **tx)
        .await?;
        return Ok((false, row, count));
    }

    sqlx::query(
        "INSERT INTO thread_members (thread_id, user_id, join_timestamp, flags, muted) VALUES (?, ?, ?, ?, FALSE)",
    )
    .bind(thread_id)
    .bind(user_id)
    .bind(now_ms)
    .bind(flags)
    .execute(&mut **tx)
    .await?;

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_members WHERE thread_id = ?")
            .bind(thread_id)
            .fetch_one(&mut **tx)
            .await?;
    let count_u32 = count.max(0) as u32;

    sqlx::query("UPDATE channel_threads SET member_count = ? WHERE channel_id = ?")
        .bind(count_u32)
        .bind(thread_id)
        .execute(&mut **tx)
        .await?;

    let inserted = ThreadMemberDbRow {
        thread_id: thread_id.to_string(),
        user_id: user_id.to_string(),
        join_timestamp: now_ms,
        flags,
        muted: false,
        mute_config: None,
    };
    Ok((true, inserted, count_u32))
}

pub(super) async fn remove_thread_member_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    thread_id: &str,
    user_id: &str,
) -> Result<Option<u32>, AppError> {
    let res = sqlx::query("DELETE FROM thread_members WHERE thread_id = ? AND user_id = ?")
        .bind(thread_id)
        .bind(user_id)
        .execute(&mut **tx)
        .await?;
    if res.rows_affected() == 0 {
        return Ok(None);
    }

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM thread_members WHERE thread_id = ?")
            .bind(thread_id)
            .fetch_one(&mut **tx)
            .await?;
    let count_u32 = count.max(0) as u32;

    sqlx::query("UPDATE channel_threads SET member_count = ? WHERE channel_id = ?")
        .bind(count_u32)
        .bind(thread_id)
        .execute(&mut **tx)
        .await?;

    Ok(Some(count_u32))
}

pub(super) async fn hydrate_thread(
    conn: &mut sqlx::MySqlConnection,
    row: ThreadDbRow,
    viewer_id: &str,
    include_member_ids: bool,
) -> Result<ThreadResponse, AppError> {
    let list = hydrate_threads_batch(conn, vec![row], viewer_id, include_member_ids).await?;
    list.into_iter()
        .next()
        .ok_or_else(AppError::unknown_channel)
}

pub(super) async fn hydrate_threads_batch(
    conn: &mut sqlx::MySqlConnection,
    rows: Vec<ThreadDbRow>,
    viewer_id: &str,
    include_member_ids: bool,
) -> Result<Vec<ThreadResponse>, AppError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let forum_thread_ids: Vec<&str> = rows
        .iter()
        .filter(|r| ChannelType::is_forum_like(r.parent_kind))
        .map(|r| r.id.as_str())
        .collect();

    let mut applied_tags_by_thread: HashMap<String, Vec<String>> = HashMap::new();
    let mut previews_by_thread: HashMap<String, Vec<String>> = HashMap::new();

    if !forum_thread_ids.is_empty() {
        let mut tqb = QueryBuilder::<MySql>::new(
            "SELECT thread_id, tag_id FROM thread_applied_tags WHERE thread_id IN (",
        );
        let mut sep = tqb.separated(", ");
        for tid in &forum_thread_ids {
            sep.push_bind(*tid);
        }
        tqb.push(") ORDER BY position ASC");
        let tag_rows: Vec<(String, String)> = tqb.build_query_as().fetch_all(&mut *conn).await?;
        for (tid, tag_id) in tag_rows {
            applied_tags_by_thread
                .entry(tid)
                .or_default()
                .push(tag_id);
        }

        let mut pqb = QueryBuilder::<MySql>::new(
            "SELECT thread_id, user_id FROM thread_members WHERE thread_id IN (",
        );
        let mut sep = pqb.separated(", ");
        for tid in &forum_thread_ids {
            sep.push_bind(*tid);
        }
        pqb.push(") ORDER BY join_timestamp DESC");
        let preview_rows: Vec<(String, String)> =
            pqb.build_query_as().fetch_all(&mut *conn).await?;
        for (tid, uid) in preview_rows {
            let entry = previews_by_thread.entry(tid).or_default();
            if entry.len() < MAX_THREAD_MEMBER_IDS_PREVIEW {
                entry.push(uid);
            }
        }
    }

    let mut mqb = QueryBuilder::<MySql>::new(
        "SELECT thread_id, user_id, join_timestamp, flags, muted, mute_config FROM thread_members WHERE user_id = ",
    );
    mqb.push_bind(viewer_id);
    mqb.push(" AND thread_id IN (");
    let mut sep = mqb.separated(", ");
    for r in &rows {
        sep.push_bind(&r.id);
    }
    mqb.push(")");

    let self_rows = mqb
        .build_query_as::<ThreadMemberDbRow>()
        .fetch_all(&mut *conn)
        .await?;
    let mut self_by_thread: HashMap<String, ThreadMember> = self_rows
        .into_iter()
        .map(|m| {
            let tid = m.thread_id.clone();
            (tid, ThreadMember::from_db_self(m, include_member_ids))
        })
        .collect();

    Ok(rows
        .into_iter()
        .map(|r| {
            let tid = r.id.clone();
            let tags = applied_tags_by_thread.remove(&tid).unwrap_or_default();
            let previews = previews_by_thread.remove(&tid).unwrap_or_default();
            let self_member = self_by_thread.remove(&tid);
            ThreadResponse::from_row(r, tags, previews, self_member)
        })
        .collect())
}
