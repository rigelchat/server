use std::collections::{HashMap, HashSet};

use sqlx::{MySql, QueryBuilder};

use crate::AppState;
use crate::constants::channel::ChannelType;
use crate::constants::flags::PermissionFlags;
use crate::constants::limits::{
    MAX_ALLOWED_MENTION_IDS, MAX_SNOWFLAKE_LENGTH, MENTION_READ_STATE_BATCH_SIZE,
};
use crate::error::{AppError, ValidationCode};
use crate::models::rest::messages::AllowedMentionsInput;
use crate::models::role::RolePerm;
use crate::models::user::{PartialUser, UserRow};
use crate::services::{permission_service, queries};
use crate::utils::snowflake::validate_snowflake;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ParsedMentions {
    pub user_ids: Vec<String>,
    pub role_ids: Vec<String>,
    pub everyone: bool,
}

#[derive(Debug, Default)]
pub struct ResolvedMentions {
    pub mentions: Vec<PartialUser>,
    pub mention_roles: Vec<String>,
    pub mention_everyone: bool,
}

fn find_closing_backticks(bytes: &[u8], after: usize, count: usize) -> Option<usize> {
    let mut j = after;
    while j < bytes.len() {
        if bytes[j] == b'`' {
            let start = j;
            while j < bytes.len() && bytes[j] == b'`' {
                j += 1;
            }
            let run = j - start;
            if (count >= 3 && run >= count) || (count < 3 && run == count) {
                return Some(j);
            }
        } else {
            j += 1;
        }
    }
    None
}

fn is_mention_boundary_byte(b: u8) -> bool {
    !b.is_ascii_alphanumeric() && b != b'_'
}

pub fn parse_content_mentions(content: &str) -> ParsedMentions {
    let mut out = ParsedMentions::default();
    let mut seen_users = HashSet::new();
    let mut seen_roles = HashSet::new();

    let bytes = content.as_bytes();
    let mut i = 0usize;

    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            if matches!(bytes[i + 1], b'\\' | b'<' | b'@' | b'`') {
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }

        if bytes[i] == b'`' {
            let start = i;
            while i < bytes.len() && bytes[i] == b'`' {
                i += 1;
            }
            let count = i - start;
            if let Some(end_pos) = find_closing_backticks(bytes, i, count) {
                i = end_pos;
            }
            continue;
        }

        if bytes[i] == b'<' && i + 2 < bytes.len() && bytes[i + 1] == b'@' {
            let mut p = i + 2;
            let is_role = if bytes[p] == b'&' {
                p += 1;
                true
            } else {
                if bytes[p] == b'!' {
                    p += 1;
                }
                false
            };

            let id_start = p;
            while p < bytes.len() && bytes[p].is_ascii_digit() {
                p += 1;
            }

            let id_len = p - id_start;
            if id_len > 0 && id_len <= MAX_SNOWFLAKE_LENGTH && p < bytes.len() && bytes[p] == b'>' {
                let id_str = &content[id_start..p];
                if id_str.parse::<u64>().is_ok() {
                    if is_role {
                        if seen_roles.insert(id_str.to_string()) {
                            out.role_ids.push(id_str.to_string());
                        }
                    } else if seen_users.insert(id_str.to_string()) {
                        out.user_ids.push(id_str.to_string());
                    }
                    i = p + 1;
                    continue;
                }
            }
        }

        if bytes[i] == b'@' && (i == 0 || is_mention_boundary_byte(bytes[i - 1])) {
            let tail = &bytes[i..];
            let keyword_len = if tail.starts_with(b"@everyone") {
                Some(9)
            } else if tail.starts_with(b"@here") {
                Some(5)
            } else {
                None
            };

            if let Some(len) = keyword_len {
                let end = i + len;
                if end == bytes.len() || is_mention_boundary_byte(bytes[end]) {
                    out.everyone = true;
                    i = end;
                    continue;
                }
            }
        }

        i += 1;
    }

    out
}

pub fn validate_allowed_mentions(am: &AllowedMentionsInput) -> Result<(), AppError> {
    let mut parse_has_users = false;
    let mut parse_has_roles = false;

    if let Some(parse) = &am.parse {
        for item in parse {
            match item.as_str() {
                "users" => parse_has_users = true,
                "roles" => parse_has_roles = true,
                "everyone" => {}
                _ => {
                    return Err(AppError::validation(
                        "allowed_mentions.parse",
                        ValidationCode::InvalidFormat,
                        "allowed_mentions.parse entries must be 'users', 'roles', or 'everyone'.",
                    ));
                }
            }
        }
    }

    if let Some(users) = &am.users {
        if parse_has_users {
            return Err(AppError::validation(
                "allowed_mentions",
                ValidationCode::InvalidFormat,
                "Cannot specify both 'users' in parse and 'users' list.",
            ));
        }
        if users.len() > MAX_ALLOWED_MENTION_IDS {
            return Err(AppError::validation(
                "allowed_mentions.users",
                ValidationCode::InvalidFormat,
                "allowed_mentions.users must contain at most 100 entries.",
            ));
        }
        for uid in users {
            validate_snowflake("allowed_mentions.users", uid)?;
        }
    }

    if let Some(roles) = &am.roles {
        if parse_has_roles {
            return Err(AppError::validation(
                "allowed_mentions",
                ValidationCode::InvalidFormat,
                "Cannot specify both 'roles' in parse and 'roles' list.",
            ));
        }
        if roles.len() > MAX_ALLOWED_MENTION_IDS {
            return Err(AppError::validation(
                "allowed_mentions.roles",
                ValidationCode::InvalidFormat,
                "allowed_mentions.roles must contain at most 100 entries.",
            ));
        }
        for rid in roles {
            validate_snowflake("allowed_mentions.roles", rid)?;
        }
    }

    Ok(())
}

pub async fn resolve_mentions(
    state: &AppState,
    guild_id: &str,
    sender_id: &str,
    sender_perms: PermissionFlags,
    content: &str,
    allowed_mentions: Option<&AllowedMentionsInput>,
    reply_author_id: Option<&str>,
) -> Result<ResolvedMentions, AppError> {
    if let Some(am) = allowed_mentions {
        validate_allowed_mentions(am)?;
    }

    let parsed = parse_content_mentions(content);
    let can_mention_everyone = sender_perms.contains(PermissionFlags::MENTION_EVERYONE);

    let (
        allow_all_users,
        allow_all_roles,
        allow_everyone,
        allowed_user_set,
        allowed_role_set,
        ping_replied_user,
    ) = match allowed_mentions {
        None => (true, true, true, HashSet::new(), HashSet::new(), true),
        Some(am) => {
            let parse = am.parse.as_deref().unwrap_or(&[]);
            let all_users = parse.iter().any(|s| s == "users");
            let all_roles = parse.iter().any(|s| s == "roles");
            let everyone = parse.iter().any(|s| s == "everyone");
            let u_set: HashSet<&str> = am
                .users
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(String::as_str)
                .collect();
            let r_set: HashSet<&str> = am
                .roles
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(String::as_str)
                .collect();
            (
                all_users,
                all_roles,
                everyone,
                u_set,
                r_set,
                am.replied_user.unwrap_or(true),
            )
        }
    };

    let mention_everyone = parsed.everyone && allow_everyone && can_mention_everyone;

    let candidate_roles: Vec<String> = parsed
        .role_ids
        .into_iter()
        .filter(|rid| {
            rid != guild_id && (allow_all_roles || allowed_role_set.contains(rid.as_str()))
        })
        .take(MAX_ALLOWED_MENTION_IDS)
        .collect();

    let mention_roles = if candidate_roles.is_empty() {
        Vec::new()
    } else {
        let mut rqb =
            QueryBuilder::<MySql>::new("SELECT id, mentionable FROM roles WHERE guild_id = ");
        rqb.push_bind(guild_id);
        rqb.push(" AND id IN (");
        let mut sep = rqb.separated(", ");
        for rid in &candidate_roles {
            sep.push_bind(rid);
        }
        rqb.push(")");

        let rows: Vec<(String, bool)> = rqb.build_query_as().fetch_all(&state.db).await?;
        let allowed_map: HashMap<String, bool> = rows.into_iter().collect();

        candidate_roles
            .into_iter()
            .filter(|rid| {
                allowed_map
                    .get(rid)
                    .is_some_and(|&mentionable| mentionable || can_mention_everyone)
            })
            .collect()
    };

    let mut candidate_users: Vec<String> = Vec::new();
    let mut seen_users = HashSet::new();

    for uid in parsed.user_ids {
        if (allow_all_users || allowed_user_set.contains(uid.as_str()))
            && seen_users.insert(uid.clone())
        {
            candidate_users.push(uid);
            if candidate_users.len() >= MAX_ALLOWED_MENTION_IDS {
                break;
            }
        }
    }

    if ping_replied_user
        && let Some(ref_uid) = reply_author_id
        && ref_uid != sender_id
        && candidate_users.len() < MAX_ALLOWED_MENTION_IDS
        && seen_users.insert(ref_uid.to_string())
    {
        candidate_users.push(ref_uid.to_string());
    }

    let mentions = if candidate_users.is_empty() {
        Vec::new()
    } else {
        let mut uqb = QueryBuilder::<MySql>::new(format!(
            "SELECT {} FROM users WHERE id IN (",
            UserRow::SELECT_COLS
        ));
        let mut sep = uqb.separated(", ");
        for uid in &candidate_users {
            sep.push_bind(uid);
        }
        uqb.push(")");

        let rows = uqb.build_query_as::<UserRow>().fetch_all(&state.db).await?;
        let by_id: HashMap<String, PartialUser> = rows
            .iter()
            .map(|r| (r.id.clone(), PartialUser::from(r)))
            .collect();

        candidate_users
            .into_iter()
            .filter_map(|uid| by_id.get(&uid).cloned())
            .collect()
    };

    Ok(ResolvedMentions {
        mentions,
        mention_roles,
        mention_everyone,
    })
}

pub async fn persist_mentions_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    message_id: &str,
    mentions: &[PartialUser],
    mention_roles: &[String],
    replace_existing: bool,
) -> Result<(), AppError> {
    if replace_existing {
        sqlx::query("DELETE FROM message_mentions WHERE message_id = ?")
            .bind(message_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("DELETE FROM message_mention_roles WHERE message_id = ?")
            .bind(message_id)
            .execute(&mut **tx)
            .await?;
    }

    if !mentions.is_empty() {
        let mut m_qb = QueryBuilder::<MySql>::new(
            "INSERT IGNORE INTO message_mentions (message_id, user_id) ",
        );
        m_qb.push_values(mentions, |mut b, u| {
            b.push_bind(message_id).push_bind(&u.id);
        });
        m_qb.build().execute(&mut **tx).await?;
    }

    if !mention_roles.is_empty() {
        let mut r_qb = QueryBuilder::<MySql>::new(
            "INSERT IGNORE INTO message_mention_roles (message_id, role_id) ",
        );
        r_qb.push_values(mention_roles, |mut b, rid| {
            b.push_bind(message_id).push_bind(rid);
        });
        r_qb.build().execute(&mut **tx).await?;
    }

    Ok(())
}

pub async fn increment_mention_counts_tx(
    tx: &mut sqlx::Transaction<'_, MySql>,
    guild_id: &str,
    channel_id: &str,
    channel_type: u32,
    parent_id: Option<&str>,
    sender_id: &str,
    resolved: &ResolvedMentions,
) -> Result<(), AppError> {
    let explicit_ids: Vec<&str> = resolved
        .mentions
        .iter()
        .filter(|u| !u.bot && u.id != sender_id)
        .map(|u| u.id.as_str())
        .collect();

    if !resolved.mention_everyone && resolved.mention_roles.is_empty() && explicit_ids.is_empty() {
        return Ok(());
    }

    let Some(owner_id) =
        sqlx::query_scalar::<_, String>("SELECT owner_id FROM guilds WHERE id = ?")
            .bind(guild_id)
            .fetch_optional(&mut **tx)
            .await?
    else {
        return Ok(());
    };

    let roles: Vec<RolePerm> =
        sqlx::query_as("SELECT id, permissions FROM roles WHERE guild_id = ?")
            .bind(guild_id)
            .fetch_all(&mut **tx)
            .await?;

    let overwrite_channel_id = if ChannelType::is_thread(channel_type) {
        parent_id.unwrap_or(channel_id)
    } else {
        channel_id
    };
    let overwrites = queries::fetch_channel_overwrites(&mut **tx, overwrite_channel_id).await?;

    let (candidate_ids, roles_by_user) = if resolved.mention_everyone {
        let members: Vec<String> = sqlx::query_scalar(
            r#"
            SELECT gm.user_id
            FROM guild_members gm
            INNER JOIN users u ON u.id = gm.user_id
            WHERE gm.guild_id = ? AND u.bot = FALSE AND gm.user_id != ?
            "#,
        )
        .bind(guild_id)
        .bind(sender_id)
        .fetch_all(&mut **tx)
        .await?;

        if members.is_empty() {
            return Ok(());
        }

        let role_map = queries::fetch_guild_member_roles_map(&mut **tx, guild_id).await?;
        (members, role_map)
    } else {
        let mut candidate_set: HashSet<String> = HashSet::new();

        if !resolved.mention_roles.is_empty() {
            let mut rqb = QueryBuilder::<MySql>::new(
                r#"
                SELECT DISTINCT gmr.user_id
                FROM guild_member_roles gmr
                INNER JOIN users u ON u.id = gmr.user_id
                WHERE gmr.guild_id = 
                "#,
            );
            rqb.push_bind(guild_id);
            rqb.push(" AND u.bot = FALSE AND gmr.user_id != ");
            rqb.push_bind(sender_id);
            rqb.push(" AND gmr.role_id IN (");
            let mut sep = rqb.separated(", ");
            for rid in &resolved.mention_roles {
                sep.push_bind(rid);
            }
            rqb.push(")");

            let role_holders: Vec<String> = rqb.build_query_scalar().fetch_all(&mut **tx).await?;
            candidate_set.extend(role_holders);
        }

        if !explicit_ids.is_empty() {
            let mut mqb = QueryBuilder::<MySql>::new(
                r#"
                SELECT gm.user_id
                FROM guild_members gm
                INNER JOIN users u ON u.id = gm.user_id
                WHERE gm.guild_id = 
                "#,
            );
            mqb.push_bind(guild_id);
            mqb.push(" AND u.bot = FALSE AND gm.user_id IN (");
            let mut sep = mqb.separated(", ");
            for uid in &explicit_ids {
                sep.push_bind(*uid);
            }
            mqb.push(")");

            let member_hits: Vec<String> = mqb.build_query_scalar().fetch_all(&mut **tx).await?;
            candidate_set.extend(member_hits);
        }

        if candidate_set.is_empty() {
            return Ok(());
        }

        let candidates: Vec<String> = candidate_set.into_iter().collect();
        let mut role_map: HashMap<String, Vec<String>> = HashMap::new();
        for chunk in candidates.chunks(MENTION_READ_STATE_BATCH_SIZE) {
            let mut mrqb = QueryBuilder::<MySql>::new(
                "SELECT user_id, role_id FROM guild_member_roles WHERE guild_id = ",
            );
            mrqb.push_bind(guild_id);
            mrqb.push(" AND role_id != ");
            mrqb.push_bind(guild_id);
            mrqb.push(" AND user_id IN (");
            let mut sep = mrqb.separated(", ");
            for uid in chunk {
                sep.push_bind(uid);
            }
            mrqb.push(")");
            let pairs: Vec<(String, String)> = mrqb.build_query_as().fetch_all(&mut **tx).await?;
            for (uid, rid) in pairs {
                role_map.entry(uid).or_default().push(rid);
            }
        }

        (candidates, role_map)
    };

    let thread_members: HashSet<String> = if channel_type == ChannelType::PrivateThread as u32 {
        sqlx::query_scalar::<_, String>("SELECT user_id FROM thread_members WHERE thread_id = ?")
            .bind(channel_id)
            .fetch_all(&mut **tx)
            .await?
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };

    let notify_ids: Vec<String> = candidate_ids
        .into_iter()
        .filter(|uid| {
            let member_roles = roles_by_user.get(uid).map(Vec::as_slice).unwrap_or(&[]);
            let base = permission_service::compute_base_permissions(
                &owner_id,
                guild_id,
                uid,
                &roles,
                member_roles,
            );
            let perms = permission_service::compute_channel_permissions(
                base,
                guild_id,
                uid,
                member_roles,
                &overwrites,
            );
            if !perms.contains(PermissionFlags::VIEW_CHANNEL) {
                return false;
            }
            if channel_type == ChannelType::PrivateThread as u32
                && !thread_members.contains(uid)
                && !perms.contains(PermissionFlags::MANAGE_THREADS)
            {
                return false;
            }
            true
        })
        .collect();

    for chunk in notify_ids.chunks(MENTION_READ_STATE_BATCH_SIZE) {
        let mut qb = QueryBuilder::<MySql>::new(
            "INSERT INTO read_states (user_id, channel_id, last_message_id, mention_count) ",
        );
        qb.push_values(chunk, |mut b, uid| {
            b.push_bind(uid)
                .push_bind(channel_id)
                .push_bind(Option::<&str>::None)
                .push_bind(1u32);
        });
        qb.push(" ON DUPLICATE KEY UPDATE mention_count = mention_count + 1");
        qb.build().execute(&mut **tx).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_role_and_everyone_mentions_ignoring_code_and_escapes() {
        let content = concat!(
            "Salut <@100000000000000001> et <@!100000000000000002> et <@100000000000000001> ",
            "ainsi que <@&200000000000000001>! ",
            "Email test@here.com ignoré, \\@everyone échappé, ",
            "`<@999999999999999999> @everyone` en code, ",
            "```rust\n<@&888888888888888888> @here\n``` ",
            "et enfin @here!"
        );

        let parsed = parse_content_mentions(content);
        assert_eq!(
            parsed.user_ids,
            vec!["100000000000000001", "100000000000000002"]
        );
        assert_eq!(parsed.role_ids, vec!["200000000000000001"]);
        assert!(parsed.everyone);
    }

    #[test]
    fn unclosed_backtick_does_not_suppress_mentions() {
        let parsed = parse_content_mentions("oops `unclosed <@123456789012345678> @everyone");
        assert_eq!(parsed.user_ids, vec!["123456789012345678"]);
        assert!(parsed.everyone);
    }

    #[test]
    fn validates_allowed_mentions_constraints() {
        assert!(validate_allowed_mentions(&AllowedMentionsInput::default()).is_ok());

        let invalid_parse = AllowedMentionsInput {
            parse: Some(vec!["channels".to_string()]),
            ..Default::default()
        };
        assert!(validate_allowed_mentions(&invalid_parse).is_err());

        let conflict = AllowedMentionsInput {
            parse: Some(vec!["users".to_string()]),
            users: Some(vec!["123456789012345678".to_string()]),
            ..Default::default()
        };
        assert!(validate_allowed_mentions(&conflict).is_err());

        let valid_empty_parse_with_users = AllowedMentionsInput {
            parse: Some(vec![]),
            users: Some(vec!["123456789012345678".to_string()]),
            ..Default::default()
        };
        assert!(validate_allowed_mentions(&valid_empty_parse_with_users).is_ok());
    }
}
