use std::collections::{HashMap, HashSet};

use sqlx::{FromRow, MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::PermissionFlags;
use crate::constants::gateway::events;
use crate::constants::limits::{MAX_READ_STATE_MENTION_COUNT, MAX_READ_STATES_ACK_BATCH};
use crate::error::{AppError, ValidationCode};
use crate::models::channel::{ChannelOverwriteRow, PermissionOverwrite};
use crate::models::gateway::events::MessageAckEvent;
use crate::models::guild::GatewayGuild;
use crate::models::read_state::{ReadState, read_state_flags_for_channel_type};
use crate::models::rest::read_states::{
    AckMessageRequest, AckMessageResponse, AckReadStatesRequest, AckReadStatesResponse,
    BulkAckReadStatesRequest,
};
use crate::models::role::RolePerm;
use crate::services::{permission_service, thread};
use crate::utils::snowflake::validate_snowflake;

#[derive(Debug, FromRow)]
struct ReadyReadStateRow {
    channel_id: String,
    last_message_id: Option<String>,
    last_pin_timestamp: Option<u64>,
    mention_count: u32,
    channel_type: u32,
    guild_id: String,
    parent_id: Option<String>,
}

#[derive(Debug, FromRow)]
struct ChannelMetaForAckRow {
    id: String,
    guild_id: String,
    kind: u32,
    parent_id: Option<String>,
}

#[derive(Debug, FromRow)]
struct StoredReadStateRow {
    channel_id: String,
    last_message_id: Option<String>,
    last_pin_timestamp: Option<u64>,
    mention_count: u32,
}

#[derive(Debug, Clone)]
struct StoredReadState {
    last_message_id: Option<String>,
    last_pin_timestamp: Option<u64>,
    mention_count: u32,
}

#[derive(Debug)]
struct ValidatedAckEntry {
    channel_id: String,
    message_id: String,
    message_id_num: u64,
    mention_count: u32,
    manual: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum AckTransition {
    Unchanged,
    Updated {
        next_last_message_id: String,
        next_mention_count: u32,
    },
}

struct AckInputItem {
    channel_id: String,
    message_id: String,
    mention_count: Option<i64>,
    manual: Option<bool>,
}

fn validate_mention_count(field: &str, raw: Option<i64>) -> Result<u32, AppError> {
    match raw {
        None => Ok(0),
        Some(v) if (0..=MAX_READ_STATE_MENTION_COUNT as i64).contains(&v) => Ok(v as u32),
        Some(_) => Err(AppError::validation(
            field,
            ValidationCode::ValueMustBeIntegerInRange,
            "mention_count must be between 0 and 2147483647.",
        )),
    }
}

fn validate_ack_entries(items: Vec<AckInputItem>) -> Result<Vec<ValidatedAckEntry>, AppError> {
    if items.is_empty() || items.len() > MAX_READ_STATES_ACK_BATCH {
        return Err(AppError::validation(
            "read_states",
            ValidationCode::InvalidFormat,
            "read_states must contain between 1 and 100 entries.",
        ));
    }

    let mut out = Vec::with_capacity(items.len());
    for (idx, item) in items.into_iter().enumerate() {
        let ch_field = format!("read_states.{idx}.channel_id");
        let msg_field = format!("read_states.{idx}.message_id");
        let mc_field = format!("read_states.{idx}.mention_count");

        validate_snowflake(&ch_field, &item.channel_id)?;
        validate_snowflake(&msg_field, &item.message_id)?;
        let message_id_num = item.message_id.parse::<u64>().map_err(|_| {
            AppError::validation(
                &msg_field,
                ValidationCode::InvalidFormat,
                format!("Invalid snowflake for '{msg_field}'."),
            )
        })?;
        let mention_count = validate_mention_count(&mc_field, item.mention_count)?;

        out.push(ValidatedAckEntry {
            channel_id: item.channel_id,
            message_id: item.message_id,
            message_id_num,
            mention_count,
            manual: item.manual.unwrap_or(false),
        });
    }

    Ok(out)
}

fn compute_ack_transition(
    current: Option<&StoredReadState>,
    message_id: &str,
    message_id_num: u64,
    mention_count: u32,
    manual: bool,
) -> AckTransition {
    let Some(cur) = current else {
        return AckTransition::Updated {
            next_last_message_id: message_id.to_string(),
            next_mention_count: mention_count,
        };
    };

    let cur_num = cur
        .last_message_id
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok());

    let should_update = if manual {
        cur_num != Some(message_id_num) || cur.mention_count != mention_count
    } else {
        match cur_num {
            None => true,
            Some(cur_id) => {
                message_id_num > cur_id
                    || (message_id_num == cur_id && cur.mention_count != mention_count)
            }
        }
    };

    if should_update {
        AckTransition::Updated {
            next_last_message_id: message_id.to_string(),
            next_mention_count: mention_count,
        }
    } else {
        AckTransition::Unchanged
    }
}

pub async fn fetch_ready_read_states(
    state: &AppState,
    user_id: &str,
    is_bot: bool,
    can_use_threads: bool,
    guilds: &[GatewayGuild],
) -> Result<Vec<ReadState>, AppError> {
    if is_bot || guilds.is_empty() {
        return Ok(Vec::new());
    }

    let mut visible_channels: HashMap<&str, u32> = HashMap::new();
    for g in guilds {
        for ch in &g.channels {
            visible_channels.insert(ch.base.id.as_str(), ch.base.kind);
        }
    }

    let rows = sqlx::query_as::<_, ReadyReadStateRow>(
        r#"
        SELECT
            rs.channel_id, rs.last_message_id, rs.last_pin_timestamp, rs.mention_count,
            c.type AS channel_type, c.guild_id, ct.parent_id
        FROM read_states rs
        INNER JOIN channels c ON c.id = rs.channel_id
        LEFT JOIN channel_threads ct ON ct.channel_id = c.id
        WHERE rs.user_id = ?
        "#,
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await?;

    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let mut private_thread_ids: Vec<&str> = Vec::new();
    if can_use_threads {
        for r in &rows {
            if r.channel_type == ChannelType::PrivateThread as u32
                && r.parent_id
                    .as_deref()
                    .is_some_and(|pid| visible_channels.contains_key(pid))
            {
                private_thread_ids.push(r.channel_id.as_str());
            }
        }
    }

    let joined_private: HashSet<String> = if private_thread_ids.is_empty() {
        HashSet::new()
    } else {
        let mut qb =
            QueryBuilder::<MySql>::new("SELECT thread_id FROM thread_members WHERE user_id = ");
        qb.push_bind(user_id);
        qb.push(" AND thread_id IN (");
        let mut sep = qb.separated(", ");
        for tid in &private_thread_ids {
            sep.push_bind(*tid);
        }
        qb.push(")");
        qb.build_query_scalar::<String>()
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .collect()
    };

    let mut mod_cache: HashMap<String, bool> = HashMap::new();
    let mut mod_conn: Option<sqlx::pool::PoolConnection<MySql>> = None;
    let mut out = Vec::with_capacity(rows.len());

    for row in rows {
        if ChannelType::is_thread(row.channel_type) {
            if !can_use_threads {
                continue;
            }
            let Some(ref parent_id) = row.parent_id else {
                continue;
            };
            if !visible_channels.contains_key(parent_id.as_str()) {
                continue;
            }
            if row.channel_type == ChannelType::PrivateThread as u32
                && !joined_private.contains(&row.channel_id)
            {
                let is_mod = match mod_cache.get(parent_id).copied() {
                    Some(v) => v,
                    None => {
                        let conn = match mod_conn.as_mut() {
                            Some(c) => c,
                            None => mod_conn.insert(state.db.acquire().await?),
                        };
                        let v = permission_service::compute_user_thread_permissions_conn(
                            conn,
                            &row.guild_id,
                            parent_id,
                            user_id,
                        )
                        .await
                        .is_ok_and(|(p, _)| p.contains(PermissionFlags::MANAGE_THREADS));
                        mod_cache.insert(parent_id.clone(), v);
                        v
                    }
                };
                if !is_mod {
                    continue;
                }
            }
            out.push(ReadState::from_parts(
                row.channel_id,
                row.mention_count,
                row.last_message_id,
                row.last_pin_timestamp,
                row.channel_type,
            ));
        } else {
            if !visible_channels.contains_key(row.channel_id.as_str())
                || !ChannelType::supports_read_states(row.channel_type)
            {
                continue;
            }
            out.push(ReadState::from_parts(
                row.channel_id,
                row.mention_count,
                row.last_message_id,
                row.last_pin_timestamp,
                row.channel_type,
            ));
        }
    }

    Ok(out)
}

async fn resolve_accessible_channels(
    state: &AppState,
    user_id: &str,
    entries: &[ValidatedAckEntry],
) -> Result<HashMap<String, u32>, AppError> {
    let mut unique_ids: Vec<&str> = entries
        .iter()
        .map(|e| e.channel_id.as_str())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    unique_ids.sort_unstable();

    if unique_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let mut cqb = QueryBuilder::<MySql>::new(
        "SELECT c.id, c.guild_id, c.type AS kind, ct.parent_id FROM channels c LEFT JOIN channel_threads ct ON ct.channel_id = c.id WHERE c.id IN (",
    );
    let mut sep = cqb.separated(", ");
    for cid in &unique_ids {
        sep.push_bind(*cid);
    }
    cqb.push(")");

    let ch_rows: Vec<ChannelMetaForAckRow> = cqb
        .build_query_as::<ChannelMetaForAckRow>()
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .filter(|r| ChannelType::supports_read_states(r.kind))
        .collect();

    if ch_rows.is_empty() {
        return Ok(HashMap::new());
    }

    let mut guild_ids: Vec<&str> = ch_rows
        .iter()
        .map(|r| r.guild_id.as_str())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    guild_ids.sort_unstable();

    let mut gqb = QueryBuilder::<MySql>::new("SELECT id, owner_id FROM guilds WHERE id IN (");
    let mut sep = gqb.separated(", ");
    for gid in &guild_ids {
        sep.push_bind(*gid);
    }
    gqb.push(")");
    let owners: HashMap<String, String> = gqb
        .build_query_as::<(String, String)>()
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .collect();

    let mut mqb = QueryBuilder::<MySql>::new("SELECT guild_id FROM guild_members WHERE user_id = ");
    mqb.push_bind(user_id);
    mqb.push(" AND guild_id IN (");
    let mut sep = mqb.separated(", ");
    for gid in &guild_ids {
        sep.push_bind(*gid);
    }
    mqb.push(")");
    let member_guilds: HashSet<String> = mqb
        .build_query_scalar::<String>()
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .collect();

    let non_owner_guilds: Vec<&str> = guild_ids
        .iter()
        .copied()
        .filter(|gid| {
            member_guilds.contains(*gid) && owners.get(*gid).is_some_and(|owner| owner != user_id)
        })
        .collect();

    let mut roles_by_guild: HashMap<String, Vec<RolePerm>> = HashMap::new();
    let mut member_roles_by_guild: HashMap<String, Vec<String>> = HashMap::new();
    let mut overwrites_by_channel: HashMap<String, Vec<PermissionOverwrite>> = HashMap::new();

    if !non_owner_guilds.is_empty() {
        let mut rqb = QueryBuilder::<MySql>::new(
            "SELECT guild_id, id, permissions FROM roles WHERE guild_id IN (",
        );
        let mut sep = rqb.separated(", ");
        for gid in &non_owner_guilds {
            sep.push_bind(*gid);
        }
        rqb.push(")");
        let role_rows: Vec<(String, String, u64)> =
            rqb.build_query_as().fetch_all(&state.db).await?;
        for (gid, rid, permissions) in role_rows {
            roles_by_guild.entry(gid).or_default().push(RolePerm {
                id: rid,
                permissions,
            });
        }

        let mut mrqb = QueryBuilder::<MySql>::new(
            "SELECT guild_id, role_id FROM guild_member_roles WHERE user_id = ",
        );
        mrqb.push_bind(user_id);
        mrqb.push(" AND role_id != guild_id AND guild_id IN (");
        let mut sep = mrqb.separated(", ");
        for gid in &non_owner_guilds {
            sep.push_bind(*gid);
        }
        mrqb.push(")");
        let mr_rows: Vec<(String, String)> = mrqb.build_query_as().fetch_all(&state.db).await?;
        for (gid, rid) in mr_rows {
            member_roles_by_guild.entry(gid).or_default().push(rid);
        }

        let mut ow_target_ids: Vec<&str> = ch_rows
            .iter()
            .filter(|r| non_owner_guilds.contains(&r.guild_id.as_str()))
            .filter_map(|r| {
                if ChannelType::is_thread(r.kind) {
                    r.parent_id.as_deref()
                } else {
                    Some(r.id.as_str())
                }
            })
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        ow_target_ids.sort_unstable();

        if !ow_target_ids.is_empty() {
            let mut oqb = QueryBuilder::<MySql>::new(
                "SELECT channel_id, type, target_id, allow, deny FROM channel_permission_overwrites WHERE channel_id IN (",
            );
            let mut sep = oqb.separated(", ");
            for cid in &ow_target_ids {
                sep.push_bind(*cid);
            }
            oqb.push(")");
            let ow_rows = oqb
                .build_query_as::<ChannelOverwriteRow>()
                .fetch_all(&state.db)
                .await?;
            for ow in ow_rows {
                overwrites_by_channel
                    .entry(ow.channel_id.clone())
                    .or_default()
                    .push(PermissionOverwrite::from(ow));
            }
        }
    }

    let mut accessible: HashMap<String, u32> = HashMap::with_capacity(ch_rows.len());
    let mut private_threads_to_check: Vec<String> = Vec::new();

    for row in ch_rows {
        let Some(owner_id) = owners.get(&row.guild_id) else {
            continue;
        };
        let is_owner = owner_id == user_id;
        if !is_owner && !member_guilds.contains(&row.guild_id) {
            continue;
        }

        let (can_view, is_mod) = if is_owner {
            (true, true)
        } else {
            let roles = roles_by_guild
                .get(&row.guild_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let member_roles = member_roles_by_guild
                .get(&row.guild_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let base = permission_service::compute_base_permissions(
                owner_id,
                &row.guild_id,
                user_id,
                roles,
                member_roles,
            );
            let ow_target = if ChannelType::is_thread(row.kind) {
                let Some(ref pid) = row.parent_id else {
                    continue;
                };
                pid.as_str()
            } else {
                row.id.as_str()
            };
            let overwrites = overwrites_by_channel
                .get(ow_target)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let perms = permission_service::compute_channel_permissions(
                base,
                &row.guild_id,
                user_id,
                member_roles,
                overwrites,
            );
            (
                perms.contains(PermissionFlags::VIEW_CHANNEL),
                perms.contains(PermissionFlags::MANAGE_THREADS),
            )
        };

        if !can_view {
            continue;
        }

        if row.kind == ChannelType::PrivateThread as u32 && !is_mod {
            private_threads_to_check.push(row.id.clone());
        }
        accessible.insert(row.id, row.kind);
    }

    if !private_threads_to_check.is_empty() {
        let mut tqb =
            QueryBuilder::<MySql>::new("SELECT thread_id FROM thread_members WHERE user_id = ");
        tqb.push_bind(user_id);
        tqb.push(" AND thread_id IN (");
        let mut sep = tqb.separated(", ");
        for tid in &private_threads_to_check {
            sep.push_bind(tid);
        }
        tqb.push(")");
        let joined: HashSet<String> = tqb
            .build_query_scalar::<String>()
            .fetch_all(&state.db)
            .await?
            .into_iter()
            .collect();

        for tid in private_threads_to_check {
            if !joined.contains(&tid) {
                accessible.remove(&tid);
            }
        }
    }

    Ok(accessible)
}

async fn dispatch_message_ack_events(
    state: &AppState,
    user_id: &str,
    ack_events: &[(u32, MessageAckEvent)],
) {
    if ack_events.is_empty() {
        return;
    }
    let sessions = state.hub.sessions_for_user(user_id);
    if sessions.is_empty() {
        return;
    }

    for (channel_type, evt) in ack_events {
        if ChannelType::requires_channel_threads_capability(*channel_type) {
            let thread_sessions: Vec<_> = sessions
                .iter()
                .filter(|s| s.has_channel_threads())
                .cloned()
                .collect();
            state.dispatch_to_sessions(&thread_sessions, events::MESSAGE_ACK, evt);
        } else {
            state.dispatch_to_sessions(&sessions, events::MESSAGE_ACK, evt);
        }
    }
}

pub async fn dispatch_message_ack(
    state: &AppState,
    user_id: &str,
    channel_type: u32,
    channel_id: &str,
    message_id: &str,
    mention_count: u32,
    manual: bool,
) {
    let evt = MessageAckEvent {
        channel_id: channel_id.to_string(),
        message_id: message_id.to_string(),
        version: "0",
        mention_count: Some(mention_count),
        manual: manual.then_some(true),
        flags: read_state_flags_for_channel_type(channel_type),
    };
    dispatch_message_ack_events(state, user_id, &[(channel_type, evt)]).await;
}

pub async fn mark_channel_read_on_send_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    user_id: &str,
    channel_id: &str,
    message_id: &str,
) -> Result<(), AppError> {
    sqlx::query(
        r#"
        INSERT INTO read_states (user_id, channel_id, last_message_id, mention_count)
        VALUES (?, ?, ?, 0)
        ON DUPLICATE KEY UPDATE
            mention_count = IF(
                last_message_id IS NULL OR CAST(? AS UNSIGNED) >= CAST(last_message_id AS UNSIGNED),
                0,
                mention_count
            ),
            last_message_id = IF(
                last_message_id IS NULL OR CAST(? AS UNSIGNED) >= CAST(last_message_id AS UNSIGNED),
                ?,
                last_message_id
            )
        "#,
    )
    .bind(user_id)
    .bind(channel_id)
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .bind(message_id)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

async fn apply_ack_batch(
    state: &AppState,
    user_id: &str,
    can_use_threads: bool,
    entries: Vec<ValidatedAckEntry>,
    collect_response: bool,
) -> Result<Vec<ReadState>, AppError> {
    let accessible = resolve_accessible_channels(state, user_id, &entries).await?;
    if accessible.is_empty() {
        return Ok(Vec::new());
    }

    let mut sorted_accessible: Vec<&str> = accessible.keys().map(String::as_str).collect();
    sorted_accessible.sort_unstable();

    let mut tx = state.db.begin().await?;

    let user_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM users WHERE id = ? FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if user_exists.is_none() {
        return Err(AppError::unauthorized());
    }

    let mut rqb = QueryBuilder::<MySql>::new(
        "SELECT channel_id, last_message_id, last_pin_timestamp, mention_count FROM read_states WHERE user_id = ",
    );
    rqb.push_bind(user_id);
    rqb.push(" AND channel_id IN (");
    let mut sep = rqb.separated(", ");
    for cid in &sorted_accessible {
        sep.push_bind(*cid);
    }
    rqb.push(")");

    let existing_rows = rqb
        .build_query_as::<StoredReadStateRow>()
        .fetch_all(&mut *tx)
        .await?;

    let mut existing_keys: HashSet<String> = HashSet::with_capacity(existing_rows.len());
    let mut states: HashMap<String, StoredReadState> = HashMap::with_capacity(existing_rows.len());
    for row in existing_rows {
        existing_keys.insert(row.channel_id.clone());
        states.insert(
            row.channel_id,
            StoredReadState {
                last_message_id: row.last_message_id,
                last_pin_timestamp: row.last_pin_timestamp,
                mention_count: row.mention_count,
            },
        );
    }

    let mut dirty_channels: HashSet<String> = HashSet::new();
    let mut ack_events: Vec<(u32, MessageAckEvent)> = Vec::new();
    let mut response_states: Vec<ReadState> = if collect_response {
        Vec::with_capacity(entries.len())
    } else {
        Vec::new()
    };

    for entry in entries {
        let Some(&channel_type) = accessible.get(&entry.channel_id) else {
            continue;
        };

        let transition = compute_ack_transition(
            states.get(&entry.channel_id),
            &entry.message_id,
            entry.message_id_num,
            entry.mention_count,
            entry.manual,
        );

        if let AckTransition::Updated {
            next_last_message_id,
            next_mention_count,
        } = transition
        {
            let prev_pin = states
                .get(&entry.channel_id)
                .and_then(|s| s.last_pin_timestamp);
            states.insert(
                entry.channel_id.clone(),
                StoredReadState {
                    last_message_id: Some(next_last_message_id.clone()),
                    last_pin_timestamp: prev_pin,
                    mention_count: next_mention_count,
                },
            );
            dirty_channels.insert(entry.channel_id.clone());
            ack_events.push((
                channel_type,
                MessageAckEvent {
                    channel_id: entry.channel_id.clone(),
                    message_id: next_last_message_id,
                    version: "0",
                    mention_count: Some(next_mention_count),
                    manual: entry.manual.then_some(true),
                    flags: read_state_flags_for_channel_type(channel_type),
                },
            ));
        }

        if collect_response
            && (can_use_threads || !ChannelType::requires_channel_threads_capability(channel_type))
        {
            let current = states.get(&entry.channel_id);
            response_states.push(ReadState::from_parts(
                entry.channel_id,
                current.map_or(0, |s| s.mention_count),
                current.and_then(|s| s.last_message_id.clone()),
                current.and_then(|s| s.last_pin_timestamp),
                channel_type,
            ));
        }
    }

    if !dirty_channels.is_empty() {
        let mut dirty_sorted: Vec<String> = dirty_channels.into_iter().collect();
        dirty_sorted.sort_unstable();

        let mut to_insert: Vec<(&str, &StoredReadState)> = Vec::new();
        for cid in &dirty_sorted {
            if let Some(st) = states.get(cid) {
                if existing_keys.contains(cid) {
                    sqlx::query(
                        "UPDATE read_states SET last_message_id = ?, mention_count = ? WHERE user_id = ? AND channel_id = ?",
                    )
                    .bind(&st.last_message_id)
                    .bind(st.mention_count)
                    .bind(user_id)
                    .bind(cid)
                    .execute(&mut *tx)
                    .await?;
                } else {
                    to_insert.push((cid.as_str(), st));
                }
            }
        }

        if !to_insert.is_empty() {
            let mut iqb = QueryBuilder::<MySql>::new(
                "INSERT INTO read_states (user_id, channel_id, last_message_id, mention_count) ",
            );
            iqb.push_values(to_insert, |mut b, (cid, st)| {
                b.push_bind(user_id)
                    .push_bind(cid)
                    .push_bind(st.last_message_id.as_deref())
                    .push_bind(st.mention_count);
            });
            iqb.build().execute(&mut *tx).await?;
        }
    }

    tx.commit().await?;

    dispatch_message_ack_events(state, user_id, &ack_events).await;

    Ok(response_states)
}

pub async fn ack_read_states(
    state: &AppState,
    user_id: &str,
    can_use_threads: bool,
    is_bot: bool,
    payload: AckReadStatesRequest,
) -> Result<AckReadStatesResponse, AppError> {
    if is_bot {
        return Err(AppError::access_denied());
    }

    let entries = validate_ack_entries(
        payload
            .read_states
            .into_iter()
            .map(|e| AckInputItem {
                channel_id: e.channel_id,
                message_id: e.message_id,
                mention_count: e.mention_count,
                manual: e.manual,
            })
            .collect(),
    )?;

    let read_states = apply_ack_batch(state, user_id, can_use_threads, entries, true).await?;
    Ok(AckReadStatesResponse { read_states })
}

pub async fn ack_read_states_bulk(
    state: &AppState,
    user_id: &str,
    can_use_threads: bool,
    is_bot: bool,
    payload: BulkAckReadStatesRequest,
) -> Result<(), AppError> {
    if is_bot {
        return Err(AppError::access_denied());
    }

    let entries = validate_ack_entries(
        payload
            .read_states
            .into_iter()
            .map(|e| AckInputItem {
                channel_id: e.channel_id,
                message_id: e.message_id,
                mention_count: None,
                manual: None,
            })
            .collect(),
    )?;

    let _ = apply_ack_batch(state, user_id, can_use_threads, entries, false).await?;
    Ok(())
}

pub async fn ack_message(
    state: &AppState,
    channel_id: &str,
    message_id: &str,
    user_id: &str,
    can_use_threads: bool,
    is_bot: bool,
    payload: AckMessageRequest,
) -> Result<AckMessageResponse, AppError> {
    if is_bot {
        return Err(AppError::access_denied());
    }

    validate_snowflake("message_id", message_id)?;
    let message_id_num = message_id.parse::<u64>().map_err(|_| {
        AppError::validation(
            "message_id",
            ValidationCode::InvalidFormat,
            "Invalid snowflake for 'message_id'.",
        )
    })?;
    let mention_count = validate_mention_count("mention_count", payload.mention_count)?;
    let manual = payload.manual.unwrap_or(false);

    let meta: Option<(String, u32)> =
        sqlx::query_as("SELECT guild_id, type FROM channels WHERE id = ?")
            .bind(channel_id)
            .fetch_optional(&state.db)
            .await?;

    let Some((guild_id, channel_type)) = meta else {
        return Err(AppError::unknown_channel());
    };

    if !can_use_threads && ChannelType::requires_channel_threads_capability(channel_type) {
        return Err(AppError::unknown_channel());
    }
    if !ChannelType::supports_read_states(channel_type) {
        return Err(AppError::invalid_channel_type());
    }

    if ChannelType::is_thread(channel_type) {
        thread::require_thread_message_access(state, channel_id, user_id).await?;
    } else {
        permission_service::require_channel_permission(
            state,
            &guild_id,
            channel_id,
            user_id,
            PermissionFlags::VIEW_CHANNEL,
        )
        .await?;
    }

    let mut tx = state.db.begin().await?;

    let user_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM users WHERE id = ? FOR UPDATE")
            .bind(user_id)
            .fetch_optional(&mut *tx)
            .await?;
    if user_exists.is_none() {
        return Err(AppError::unauthorized());
    }

    let existing = sqlx::query_as::<_, StoredReadStateRow>(
        "SELECT channel_id, last_message_id, last_pin_timestamp, mention_count FROM read_states WHERE user_id = ? AND channel_id = ?",
    )
    .bind(user_id)
    .bind(channel_id)
    .fetch_optional(&mut *tx)
    .await?;

    let cur_state = existing.as_ref().map(|r| StoredReadState {
        last_message_id: r.last_message_id.clone(),
        last_pin_timestamp: r.last_pin_timestamp,
        mention_count: r.mention_count,
    });

    let transition = compute_ack_transition(
        cur_state.as_ref(),
        message_id,
        message_id_num,
        mention_count,
        manual,
    );

    let ack_event = match transition {
        AckTransition::Unchanged => None,
        AckTransition::Updated {
            next_last_message_id,
            next_mention_count,
        } => {
            if existing.is_some() {
                sqlx::query(
                    "UPDATE read_states SET last_message_id = ?, mention_count = ? WHERE user_id = ? AND channel_id = ?",
                )
                .bind(&next_last_message_id)
                .bind(next_mention_count)
                .bind(user_id)
                .bind(channel_id)
                .execute(&mut *tx)
                .await?;
            } else {
                sqlx::query(
                    "INSERT INTO read_states (user_id, channel_id, last_message_id, mention_count) VALUES (?, ?, ?, ?)",
                )
                .bind(user_id)
                .bind(channel_id)
                .bind(&next_last_message_id)
                .bind(next_mention_count)
                .execute(&mut *tx)
                .await?;
            }

            Some((
                channel_type,
                MessageAckEvent {
                    channel_id: channel_id.to_string(),
                    message_id: next_last_message_id,
                    version: "0",
                    mention_count: Some(next_mention_count),
                    manual: manual.then_some(true),
                    flags: read_state_flags_for_channel_type(channel_type),
                },
            ))
        }
    };

    tx.commit().await?;

    if let Some(evt) = ack_event {
        dispatch_message_ack_events(state, user_id, &[evt]).await;
    }

    Ok(AckMessageResponse { token: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ack_transition_respects_manual_and_monotonicity() {
        let existing = StoredReadState {
            last_message_id: Some("200".into()),
            last_pin_timestamp: None,
            mention_count: 3,
        };

        assert_eq!(
            compute_ack_transition(None, "100", 100, 0, false),
            AckTransition::Updated {
                next_last_message_id: "100".into(),
                next_mention_count: 0,
            }
        );

        assert_eq!(
            compute_ack_transition(Some(&existing), "100", 100, 0, false),
            AckTransition::Unchanged
        );

        assert_eq!(
            compute_ack_transition(Some(&existing), "200", 200, 0, false),
            AckTransition::Updated {
                next_last_message_id: "200".into(),
                next_mention_count: 0,
            }
        );

        assert_eq!(
            compute_ack_transition(Some(&existing), "200", 200, 3, false),
            AckTransition::Unchanged
        );

        assert_eq!(
            compute_ack_transition(Some(&existing), "250", 250, 0, false),
            AckTransition::Updated {
                next_last_message_id: "250".into(),
                next_mention_count: 0,
            }
        );

        assert_eq!(
            compute_ack_transition(Some(&existing), "100", 100, 1, true),
            AckTransition::Updated {
                next_last_message_id: "100".into(),
                next_mention_count: 1,
            }
        );
    }

    #[test]
    fn validates_ack_batch_limits_and_mention_count() {
        assert!(validate_ack_entries(vec![]).is_err());

        let too_many: Vec<AckInputItem> = (0..=MAX_READ_STATES_ACK_BATCH)
            .map(|i| AckInputItem {
                channel_id: (100 + i).to_string(),
                message_id: (200 + i).to_string(),
                mention_count: None,
                manual: None,
            })
            .collect();
        assert!(validate_ack_entries(too_many).is_err());

        let bad_mentions = vec![AckInputItem {
            channel_id: "100".into(),
            message_id: "200".into(),
            mention_count: Some(-1),
            manual: None,
        }];
        assert!(validate_ack_entries(bad_mentions).is_err());

        let over_mentions = vec![AckInputItem {
            channel_id: "100".into(),
            message_id: "200".into(),
            mention_count: Some(i32::MAX as i64 + 1),
            manual: None,
        }];
        assert!(validate_ack_entries(over_mentions).is_err());

        let valid = validate_ack_entries(vec![AckInputItem {
            channel_id: "100".into(),
            message_id: "200".into(),
            mention_count: Some(i32::MAX as i64),
            manual: Some(true),
        }])
        .unwrap();
        assert_eq!(valid.len(), 1);
        assert_eq!(valid[0].mention_count, MAX_READ_STATE_MENTION_COUNT);
        assert!(valid[0].manual);
    }
}
