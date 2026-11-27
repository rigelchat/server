use std::collections::{HashMap, HashSet};

use chrono::DateTime;
use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::{ChannelType, DEFAULT_FORUM_TAG_SETTING, VALID_FORUM_TAG_SETTINGS};
use crate::constants::flags::PermissionFlags;
use crate::constants::limits::{
    DEFAULT_THREADS_PAGE_LIMIT, MAX_ARCHIVED_THREADS_PAGE_LIMIT, MAX_SEARCH_QUERY_LENGTH,
    MAX_SEARCH_THREADS_LIMIT, MAX_SEARCH_THREADS_OFFSET, MIN_ARCHIVED_THREADS_PAGE_LIMIT,
};
use crate::error::{AppError, ValidationCode};
use crate::models::message::MessageRow;
use crate::models::rest::threads::{
    ListArchivedThreadsQuery, SearchThreadsQuery, SearchThreadsResponse, ThreadListResponse,
};
use crate::models::thread::{ThreadMember, ThreadMemberRow, ThreadRow};
use crate::services::{member_service, permission_service};
use crate::utils::snowflake::{validate_snowflake, validate_snowflake_opt};

use super::helpers::hydrate_threads_batch;

async fn fetch_self_members_for_threads(
    conn: &mut sqlx::MySqlConnection,
    user_id: &str,
    thread_ids: &[String],
) -> Result<Vec<ThreadMember>, AppError> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut qb = QueryBuilder::<MySql>::new(
        "SELECT thread_id, user_id, join_timestamp, flags, muted, mute_config FROM thread_members WHERE user_id = ",
    );
    qb.push_bind(user_id);
    qb.push(" AND thread_id IN (");
    let mut sep = qb.separated(", ");
    for tid in thread_ids {
        sep.push_bind(tid);
    }
    qb.push(")");
    let rows = qb
        .build_query_as::<ThreadMemberRow>()
        .fetch_all(conn)
        .await?;
    Ok(rows
        .into_iter()
        .map(|r| ThreadMember::from_db_self(r, true))
        .collect())
}

pub async fn list_active_guild_threads(
    state: &AppState,
    guild_id: &str,
    user_id: &str,
    is_bot: bool,
) -> Result<ThreadListResponse, AppError> {
    if !is_bot {
        return Err(AppError::not_found());
    }
    member_service::require_member(state, guild_id, user_id).await?;

    let mut conn = state.db.acquire().await?;
    let sql = format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE ct.guild_id = ? AND ct.archived = FALSE ORDER BY CAST(c.id AS UNSIGNED) DESC",
        ThreadRow::SELECT_COLS
    );
    let rows = sqlx::query_as::<_, ThreadRow>(&sql)
        .bind(guild_id)
        .fetch_all(&mut *conn)
        .await?;

    let mut visible_rows = Vec::with_capacity(rows.len());
    let mut perm_cache: HashMap<String, PermissionFlags> = HashMap::new();
    for r in rows {
        let perms = match perm_cache.get(&r.parent_id).copied() {
            Some(p) => p,
            None => {
                let (p, _) = permission_service::compute_user_thread_permissions_conn(
                    &mut conn,
                    guild_id,
                    &r.parent_id,
                    user_id,
                )
                .await
                .unwrap_or((PermissionFlags::empty(), PermissionFlags::empty()));
                perm_cache.insert(r.parent_id.clone(), p);
                p
            }
        };
        if !perms.contains(PermissionFlags::VIEW_CHANNEL) {
            continue;
        }
        if r.kind == ChannelType::PrivateThread as u32
            && !perms.contains(PermissionFlags::MANAGE_THREADS)
        {
            continue;
        }
        visible_rows.push(r);
    }

    let ids: Vec<String> = visible_rows.iter().map(|r| r.id.clone()).collect();
    let threads = hydrate_threads_batch(&mut conn, visible_rows, user_id, false).await?;
    let members = fetch_self_members_for_threads(&mut conn, user_id, &ids).await?;

    Ok(ThreadListResponse {
        threads,
        members,
        has_more: None,
    })
}

async fn check_archived_parent_access(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    require_moderator: bool,
    require_text_only: bool,
) -> Result<(), AppError> {
    let meta: Option<(String, u32)> = sqlx::query_as(
        "SELECT guild_id, type FROM channels WHERE id = ? AND type NOT IN (10, 11, 12)",
    )
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((guild_id, kind)) = meta else {
        return Err(AppError::not_found());
    };

    if require_text_only {
        if kind != ChannelType::GuildText as u32 {
            return Err(AppError::invalid_channel_type());
        }
    } else if !ChannelType::supports_thread_defaults(kind) {
        return Err(AppError::invalid_channel_type());
    }

    let mut required = PermissionFlags::VIEW_CHANNEL | PermissionFlags::READ_MESSAGE_HISTORY;
    if require_moderator {
        required |= PermissionFlags::MANAGE_THREADS;
    }
    permission_service::require_channel_permission(state, &guild_id, channel_id, user_id, required)
        .await?;
    Ok(())
}

fn parse_before_timestamp(before: Option<&str>) -> Result<Option<u64>, AppError> {
    let Some(raw) = before.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let dt = DateTime::parse_from_rfc3339(raw).map_err(|_| {
        AppError::validation(
            "before",
            ValidationCode::InvalidIsoTimestamp,
            "Invalid ISO8601 timestamp for 'before'.",
        )
    })?;
    Ok(Some(dt.timestamp_millis().max(0) as u64))
}

pub async fn list_public_archived_threads(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    query: ListArchivedThreadsQuery,
) -> Result<ThreadListResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }
    check_archived_parent_access(state, channel_id, user_id, false, false).await?;
    let before_ms = parse_before_timestamp(query.before.as_deref())?;
    let limit = query.limit.unwrap_or(DEFAULT_THREADS_PAGE_LIMIT).clamp(
        MIN_ARCHIVED_THREADS_PAGE_LIMIT,
        MAX_ARCHIVED_THREADS_PAGE_LIMIT,
    );

    let mut qb = QueryBuilder::<MySql>::new(format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE ct.parent_id = ",
        ThreadRow::SELECT_COLS
    ));
    qb.push_bind(channel_id);
    qb.push(" AND ct.archived = TRUE AND c.type IN (10, 11)");
    if let Some(ts) = before_ms {
        qb.push(" AND ct.archive_timestamp < ");
        qb.push_bind(ts);
    }
    qb.push(" ORDER BY ct.archive_timestamp DESC, CAST(c.id AS UNSIGNED) DESC LIMIT ");
    qb.push_bind(limit + 1);

    let mut conn = state.db.acquire().await?;
    let mut rows = qb
        .build_query_as::<ThreadRow>()
        .fetch_all(&mut *conn)
        .await?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);

    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let threads = hydrate_threads_batch(&mut conn, rows, user_id, false).await?;
    let members = fetch_self_members_for_threads(&mut conn, user_id, &ids).await?;

    Ok(ThreadListResponse {
        threads,
        members,
        has_more: Some(has_more),
    })
}

pub async fn list_private_archived_threads(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    query: ListArchivedThreadsQuery,
) -> Result<ThreadListResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }
    check_archived_parent_access(state, channel_id, user_id, true, true).await?;
    let before_ms = parse_before_timestamp(query.before.as_deref())?;
    let limit = query.limit.unwrap_or(DEFAULT_THREADS_PAGE_LIMIT).clamp(
        MIN_ARCHIVED_THREADS_PAGE_LIMIT,
        MAX_ARCHIVED_THREADS_PAGE_LIMIT,
    );

    let mut qb = QueryBuilder::<MySql>::new(format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE ct.parent_id = ",
        ThreadRow::SELECT_COLS
    ));
    qb.push_bind(channel_id);
    qb.push(" AND ct.archived = TRUE AND c.type = 12");
    if let Some(ts) = before_ms {
        qb.push(" AND ct.archive_timestamp < ");
        qb.push_bind(ts);
    }
    qb.push(" ORDER BY ct.archive_timestamp DESC, CAST(c.id AS UNSIGNED) DESC LIMIT ");
    qb.push_bind(limit + 1);

    let mut conn = state.db.acquire().await?;
    let mut rows = qb
        .build_query_as::<ThreadRow>()
        .fetch_all(&mut *conn)
        .await?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);

    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let threads = hydrate_threads_batch(&mut conn, rows, user_id, false).await?;
    let members = fetch_self_members_for_threads(&mut conn, user_id, &ids).await?;

    Ok(ThreadListResponse {
        threads,
        members,
        has_more: Some(has_more),
    })
}

pub async fn list_joined_private_archived_threads(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    query: ListArchivedThreadsQuery,
) -> Result<ThreadListResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }
    validate_snowflake_opt("before", query.before.as_deref())?;
    check_archived_parent_access(state, channel_id, user_id, false, true).await?;
    let limit = query.limit.unwrap_or(DEFAULT_THREADS_PAGE_LIMIT).clamp(
        MIN_ARCHIVED_THREADS_PAGE_LIMIT,
        MAX_ARCHIVED_THREADS_PAGE_LIMIT,
    );

    let mut qb = QueryBuilder::<MySql>::new(format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id INNER JOIN thread_members tm ON tm.thread_id = c.id WHERE ct.parent_id = ",
        ThreadRow::SELECT_COLS
    ));
    qb.push_bind(channel_id);
    qb.push(" AND tm.user_id = ");
    qb.push_bind(user_id);
    qb.push(" AND ct.archived = TRUE AND c.type = 12");
    if let Some(before_id) = &query.before {
        qb.push(" AND CAST(c.id AS UNSIGNED) < CAST(");
        qb.push_bind(before_id);
        qb.push(" AS UNSIGNED)");
    }
    qb.push(" ORDER BY CAST(c.id AS UNSIGNED) DESC LIMIT ");
    qb.push_bind(limit + 1);

    let mut conn = state.db.acquire().await?;
    let mut rows = qb
        .build_query_as::<ThreadRow>()
        .fetch_all(&mut *conn)
        .await?;
    let has_more = rows.len() > limit as usize;
    rows.truncate(limit as usize);

    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let threads = hydrate_threads_batch(&mut conn, rows, user_id, false).await?;
    let members = fetch_self_members_for_threads(&mut conn, user_id, &ids).await?;

    Ok(ThreadListResponse {
        threads,
        members,
        has_more: Some(has_more),
    })
}

fn push_search_filters<'a>(
    qb: &mut QueryBuilder<'a, MySql>,
    channel_id: &'a str,
    user_id: &'a str,
    is_moderator: bool,
    tag_setting: &'a str,
    unique_tags: &'a [String],
    q: &'a SearchThreadsQuery,
) {
    qb.push(" WHERE ct.parent_id = ");
    qb.push_bind(channel_id);

    if !is_moderator {
        qb.push(" AND (c.type != 12 OR EXISTS (SELECT 1 FROM thread_members tm WHERE tm.thread_id = c.id AND tm.user_id = ");
        qb.push_bind(user_id);
        qb.push("))");
    }

    if let Some(archived) = q.archived {
        qb.push(" AND ct.archived = ");
        qb.push_bind(archived);
    }

    if let Some(ref raw_name) = q.name {
        let trimmed: String = raw_name
            .trim()
            .chars()
            .take(MAX_SEARCH_QUERY_LENGTH)
            .collect();
        if !trimmed.is_empty() {
            let escaped = trimmed
                .to_ascii_lowercase()
                .replace('!', "!!")
                .replace('%', "!%")
                .replace('_', "!_");
            let pattern = format!("%{escaped}%");
            qb.push(" AND LOWER(c.name) LIKE ");
            qb.push_bind(pattern);
            qb.push(" ESCAPE '!'");
        }
    }

    if !unique_tags.is_empty() {
        if tag_setting == "match_all" {
            qb.push(" AND (SELECT COUNT(DISTINCT tat.tag_id) FROM thread_applied_tags tat WHERE tat.thread_id = c.id AND tat.tag_id IN (");
            let mut sep = qb.separated(", ");
            for tid in unique_tags {
                sep.push_bind(tid);
            }
            qb.push(")) = ");
            qb.push_bind(unique_tags.len() as i64);
        } else {
            qb.push(" AND EXISTS (SELECT 1 FROM thread_applied_tags tat WHERE tat.thread_id = c.id AND tat.tag_id IN (");
            let mut sep = qb.separated(", ");
            for tid in unique_tags {
                sep.push_bind(tid);
            }
            qb.push("))");
        }
    }

    if let Some(ref max_id) = q.max_id {
        qb.push(" AND CAST(c.id AS UNSIGNED) < CAST(");
        qb.push_bind(max_id);
        qb.push(" AS UNSIGNED)");
    }

    if let Some(ref min_id) = q.min_id {
        qb.push(" AND CAST(c.id AS UNSIGNED) > CAST(");
        qb.push_bind(min_id);
        qb.push(" AS UNSIGNED)");
    }
}

pub async fn search_threads(
    state: &AppState,
    channel_id: &str,
    user_id: &str,
    can_use_threads: bool,
    q: SearchThreadsQuery,
) -> Result<SearchThreadsResponse, AppError> {
    if !can_use_threads {
        return Err(AppError::not_found());
    }

    validate_snowflake_opt("max_id", q.max_id.as_deref())?;
    validate_snowflake_opt("min_id", q.min_id.as_deref())?;

    let mut unique_tags = Vec::with_capacity(q.tags.len());
    let mut seen_tags = HashSet::with_capacity(q.tags.len());
    for t in &q.tags {
        validate_snowflake("tag", t)?;
        if seen_tags.insert(t.clone()) {
            unique_tags.push(t.clone());
        }
    }

    let meta: Option<(String, u32, String)> = sqlx::query_as(
        "SELECT guild_id, type, default_tag_setting FROM channels WHERE id = ? AND type NOT IN (10, 11, 12)",
    )
    .bind(channel_id)
    .fetch_optional(&state.db)
    .await?;
    let Some((guild_id, kind, default_tag_setting)) = meta else {
        return Err(AppError::not_found());
    };

    if !ChannelType::supports_thread_defaults(kind) {
        return Err(AppError::invalid_channel_type());
    }

    let perms = permission_service::require_channel_permission(
        state,
        &guild_id,
        channel_id,
        user_id,
        PermissionFlags::VIEW_CHANNEL | PermissionFlags::READ_MESSAGE_HISTORY,
    )
    .await?;
    let is_moderator = perms.contains(PermissionFlags::MANAGE_THREADS);

    let tag_setting = q
        .tag_setting
        .as_deref()
        .unwrap_or(default_tag_setting.as_str());
    let tag_setting = if VALID_FORUM_TAG_SETTINGS.contains(&tag_setting) {
        tag_setting
    } else {
        DEFAULT_FORUM_TAG_SETTING
    };

    let sort_expr = match q.sort_by.as_deref() {
        Some("archive_time") => "ct.archive_timestamp",
        Some("creation_time") | Some("relevance") => "ct.create_timestamp",
        Some("last_message_time") | None => {
            "COALESCE(CAST(c.last_message_id AS UNSIGNED), CAST(c.id AS UNSIGNED))"
        }
        Some(_) => {
            return Err(AppError::validation(
                "sort_by",
                ValidationCode::InvalidFormat,
                "Invalid sort_by value.",
            ));
        }
    };
    let sort_dir = match q.sort_order.as_deref() {
        Some("asc") => "ASC",
        Some("desc") | None => "DESC",
        Some(_) => {
            return Err(AppError::validation(
                "sort_order",
                ValidationCode::InvalidFormat,
                "sort_order must be 'asc' or 'desc'.",
            ));
        }
    };

    let limit = q
        .limit
        .unwrap_or(MAX_SEARCH_THREADS_LIMIT)
        .clamp(1, MAX_SEARCH_THREADS_LIMIT);
    let offset = q.offset.unwrap_or(0).min(MAX_SEARCH_THREADS_OFFSET);

    let mut count_qb = QueryBuilder::<MySql>::new(
        "SELECT COUNT(*) FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id",
    );
    push_search_filters(
        &mut count_qb,
        channel_id,
        user_id,
        is_moderator,
        tag_setting,
        &unique_tags,
        &q,
    );

    let mut conn = state.db.acquire().await?;
    let total_results: i64 = count_qb.build_query_scalar().fetch_one(&mut *conn).await?;
    let total_u64 = total_results.max(0) as u64;

    if total_u64 == 0 || (offset as u64) >= total_u64 {
        return Ok(SearchThreadsResponse {
            threads: vec![],
            members: vec![],
            has_more: false,
            total_results: total_u64,
            first_messages: ChannelType::is_forum_like(kind).then_some(vec![]),
        });
    }

    let mut list_qb = QueryBuilder::<MySql>::new(format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id",
        ThreadRow::SELECT_COLS
    ));
    push_search_filters(
        &mut list_qb,
        channel_id,
        user_id,
        is_moderator,
        tag_setting,
        &unique_tags,
        &q,
    );
    list_qb.push(" ORDER BY ");
    list_qb.push(sort_expr);
    list_qb.push(" ");
    list_qb.push(sort_dir);
    list_qb.push(", CAST(c.id AS UNSIGNED) ");
    list_qb.push(sort_dir);
    list_qb.push(" LIMIT ");
    list_qb.push_bind(limit);
    list_qb.push(" OFFSET ");
    list_qb.push_bind(offset);

    let rows = list_qb
        .build_query_as::<ThreadRow>()
        .fetch_all(&mut *conn)
        .await?;
    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let threads = hydrate_threads_batch(&mut conn, rows, user_id, false).await?;
    let members = fetch_self_members_for_threads(&mut conn, user_id, &ids).await?;

    let first_messages = if ChannelType::is_forum_like(kind) {
        if ids.is_empty() {
            Some(vec![])
        } else {
            let mut mqb =
                QueryBuilder::<MySql>::new(format!("{} WHERE m.id IN (", MessageRow::SELECT_FROM));
            let mut sep = mqb.separated(", ");
            for tid in &ids {
                sep.push_bind(tid);
            }
            mqb.push(") AND m.channel_id = m.id");
            let msg_rows = mqb
                .build_query_as::<MessageRow>()
                .fetch_all(&mut *conn)
                .await?;
            Some(
                crate::services::message_service::hydrate_messages(
                    state,
                    msg_rows,
                    user_id,
                    can_use_threads,
                )
                .await?,
            )
        }
    } else {
        None
    };

    let has_more = (offset as u64 + threads.len() as u64) < total_u64;

    Ok(SearchThreadsResponse {
        threads,
        members,
        has_more,
        total_results: total_u64,
        first_messages,
    })
}
