use std::collections::HashMap;

use chrono::Utc;
use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::{PermissionFlags, ThreadMemberFlags};
use crate::constants::gateway::events;
use crate::error::AppError;
use crate::gateway::handlers::presence;
use crate::models::gateway::events::{ThreadCreateEvent, ThreadMembersUpdateEvent};
use crate::models::guild_member::GuildMemberUserRow;
use crate::models::thread::{ThreadDbRow, ThreadMember, ThreadResponse};
use crate::services::{permission_service, queries};

use super::helpers::{
    add_thread_member_tx, check_thread_view_access, ensure_active_thread_capacity,
    fetch_thread_member_self, fetch_thread_row, hydrate_thread, hydrate_threads_batch,
};

pub struct ThreadMessageContext {
    pub row: ThreadDbRow,
    pub parent_perms: PermissionFlags,
    pub thread_perms: PermissionFlags,
    pub is_moderator: bool,
}

pub async fn require_thread_message_access(
    state: &AppState,
    thread_id: &str,
    user_id: &str,
) -> Result<ThreadMessageContext, AppError> {
    let mut conn = state.db.acquire().await?;
    let row = fetch_thread_row(&mut *conn, thread_id)
        .await?
        .ok_or_else(AppError::unknown_channel)?;
    let (parent_perms, thread_perms, _) =
        check_thread_view_access(&mut conn, &row, user_id).await?;
    let is_moderator = parent_perms.contains(PermissionFlags::MANAGE_THREADS);
    Ok(ThreadMessageContext {
        row,
        parent_perms,
        thread_perms,
        is_moderator,
    })
}

pub async fn apply_thread_message_created(
    tx: &mut sqlx::Transaction<'_, MySql>,
    ctx: &ThreadMessageContext,
    user_id: &str,
    message_id: &str,
    now_ms: u64,
    is_bot: bool,
) -> Result<(bool, Option<(ThreadMember, u32)>), AppError> {
    let mut unarchived = false;
    if ctx.row.archived {
        ensure_active_thread_capacity(tx, &ctx.row.guild_id).await?;
        sqlx::query(
            "UPDATE channel_threads SET archived = FALSE, archive_timestamp = ? WHERE channel_id = ?",
        )
        .bind(now_ms)
        .bind(&ctx.row.id)
        .execute(&mut **tx)
        .await?;
        unarchived = true;
    }

    sqlx::query(
        "UPDATE channel_threads SET message_count = message_count + 1, total_message_sent = total_message_sent + 1 WHERE channel_id = ?",
    )
    .bind(&ctx.row.id)
    .execute(&mut **tx)
    .await?;

    sqlx::query("UPDATE channels SET last_message_id = ? WHERE id = ?")
        .bind(message_id)
        .bind(&ctx.row.id)
        .execute(&mut **tx)
        .await?;

    let added_info = if let Some(existing) =
        fetch_thread_member_self(&mut **tx, &ctx.row.id, user_id).await?
    {
        if !is_bot && (existing.flags & ThreadMemberFlags::HAS_INTERACTED.bits()) == 0 {
            sqlx::query("UPDATE thread_members SET flags = ? WHERE thread_id = ? AND user_id = ?")
                .bind(existing.flags | ThreadMemberFlags::HAS_INTERACTED.bits())
                .bind(&ctx.row.id)
                .bind(user_id)
                .execute(&mut **tx)
                .await?;
        }
        None
    } else {
        let member_flags = if is_bot {
            0
        } else {
            ThreadMemberFlags::HAS_INTERACTED.bits()
        };
        let (_, inserted_row, member_count) =
            add_thread_member_tx(tx, &ctx.row.id, user_id, member_flags, now_ms).await?;
        Some((
            ThreadMember::from_db_public(inserted_row, None),
            member_count,
        ))
    };

    Ok((unarchived, added_info))
}

pub async fn dispatch_thread_after_message(
    state: &AppState,
    ctx: &ThreadMessageContext,
    user_id: &str,
    unarchived: bool,
    added_info: Option<(ThreadMember, u32)>,
) {
    let mut conn = match state.db.acquire().await {
        Ok(c) => c,
        Err(_) => return,
    };
    let Ok(Some(row)) = fetch_thread_row(&mut *conn, &ctx.row.id).await else {
        return;
    };
    let Ok(hydrated_for_user) = hydrate_thread(&mut conn, row, user_id, false).await else {
        return;
    };
    drop(conn);

    if unarchived || added_info.is_some() {
        let mut broadcast = hydrated_for_user.clone();
        broadcast.member = None;
        permission_service::dispatch_channel_with_ctx(
            state,
            &ctx.row.guild_id,
            &ctx.row.parent_id,
            None,
            events::THREAD_UPDATE,
            &broadcast,
        )
        .await;
    }

    if let Some((mut added_member, member_count)) = added_info {
        let gm = sqlx::query_as::<_, GuildMemberUserRow>(
            r#"
            SELECT
                gm.user_id, gm.joined_at, gm.nick,
                u.username, u.discriminator, u.global_name, u.avatar, u.bot, u.public_flags
            FROM guild_members gm
            INNER JOIN users u ON u.id = gm.user_id
            WHERE gm.guild_id = ? AND gm.user_id = ?
            "#,
        )
        .bind(&ctx.row.guild_id)
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .map(|mr| GuildMemberUserRow::into_member_response(mr, Vec::new()));
        let gm = match gm {
            Some(mut m) => {
                m.member.roles = queries::fetch_member_roles(&state.db, &ctx.row.guild_id, user_id)
                    .await
                    .unwrap_or_default();
                Some(m)
            }
            None => None,
        };
        let pres = presence::build_presence_payloads(state, &[user_id.to_string()], None)
            .await
            .into_iter()
            .find(|p| p.status != "offline");
        added_member.member = Some(gm);
        added_member.presence = Some(pres);

        permission_service::dispatch_channel_with_ctx(
            state,
            &ctx.row.guild_id,
            &ctx.row.parent_id,
            None,
            events::THREAD_MEMBERS_UPDATE,
            &ThreadMembersUpdateEvent {
                id: ctx.row.id.clone(),
                guild_id: ctx.row.guild_id.clone(),
                member_count,
                added_members: Some(vec![added_member]),
                removed_member_ids: None,
            },
        )
        .await;

        state
            .dispatch_user(
                events::THREAD_CREATE,
                &ThreadCreateEvent {
                    thread: hydrated_for_user,
                    newly_created: None,
                },
                user_id,
            )
            .await;
    }
}

pub async fn apply_thread_message_deleted(
    tx: &mut sqlx::Transaction<'_, MySql>,
    ctx: &ThreadMessageContext,
    deleted_message_id: &str,
) -> Result<(), AppError> {
    let is_forum_first_msg =
        ChannelType::is_forum_like(ctx.row.parent_kind) && deleted_message_id == ctx.row.id;
    if !is_forum_first_msg {
        sqlx::query(
            "UPDATE channel_threads SET message_count = IF(message_count > 0, message_count - 1, 0) WHERE channel_id = ?",
        )
        .bind(&ctx.row.id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn fetch_threads_for_messages(
    state: &AppState,
    thread_ids: &[String],
    viewer_id: &str,
) -> Result<HashMap<String, ThreadResponse>, AppError> {
    if thread_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let mut qb = QueryBuilder::<MySql>::new(format!(
        "SELECT {} FROM channels c INNER JOIN channel_threads ct ON ct.channel_id = c.id INNER JOIN channels p ON p.id = ct.parent_id WHERE c.id IN (",
        ThreadDbRow::SELECT_COLS
    ));
    let mut sep = qb.separated(", ");
    for tid in thread_ids {
        sep.push_bind(tid);
    }
    qb.push(")");

    let mut conn = state.db.acquire().await?;
    let rows = qb
        .build_query_as::<ThreadDbRow>()
        .fetch_all(&mut *conn)
        .await?;
    let hydrated = hydrate_threads_batch(&mut conn, rows, viewer_id, false).await?;
    Ok(hydrated.into_iter().map(|t| (t.id.clone(), t)).collect())
}

pub fn now_ms_u64() -> u64 {
    Utc::now().timestamp_millis().max(0) as u64
}
