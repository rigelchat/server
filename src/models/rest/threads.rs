use serde::{Deserialize, Serialize};

use crate::constants::limits::{
    MAX_SEARCH_THREADS_LIMIT, MAX_SEARCH_THREADS_OFFSET, MAX_SEARCH_THREADS_TAGS,
};
use crate::error::{AppError, ValidationCode};
use crate::models::message::Message;
use crate::models::serde_helpers::deserialize_double_option;
use crate::models::thread::{ThreadMember, ThreadResponse};

#[derive(Debug, Deserialize)]
pub struct StartThreadFromMessageRequest {
    pub name: String,
    pub auto_archive_duration: Option<u32>,
    pub rate_limit_per_user: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct StartThreadMessageInput {
    pub content: Option<String>,
    pub flags: Option<u32>,
    pub allowed_mentions: Option<crate::models::rest::messages::AllowedMentionsInput>,
}

#[derive(Debug, Deserialize)]
pub struct StartThreadRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: Option<u32>,
    pub auto_archive_duration: Option<u32>,
    pub rate_limit_per_user: Option<u32>,
    pub invitable: Option<bool>,
    pub message: Option<StartThreadMessageInput>,
    pub applied_tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListArchivedThreadsQuery {
    pub before: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct ThreadListResponse {
    pub threads: Vec<ThreadResponse>,
    pub members: Vec<ThreadMember>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct SearchThreadsQuery {
    pub name: Option<String>,
    pub tags: Vec<String>,
    pub tag_setting: Option<String>,
    pub archived: Option<bool>,
    pub sort_by: Option<String>,
    pub sort_order: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub max_id: Option<String>,
    pub min_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchThreadsResponse {
    pub threads: Vec<ThreadResponse>,
    pub members: Vec<ThreadMember>,
    pub has_more: bool,
    pub total_results: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_messages: Option<Vec<Message>>,
}

#[derive(Debug, Deserialize, Default)]
pub struct GetThreadMemberQuery {
    pub with_member: Option<bool>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ListThreadMembersQuery {
    pub with_member: Option<bool>,
    pub after: Option<String>,
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ModifyThreadMemberSettingsRequest {
    pub flags: Option<u32>,
    pub muted: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_double_option")]
    pub mute_config: Option<Option<serde_json::Value>>,
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(val) = u8::from_str_radix(hex, 16) {
                    out.push(val);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_search_threads_query(raw: Option<&str>) -> Result<SearchThreadsQuery, AppError> {
    let mut out = SearchThreadsQuery::default();
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(out);
    };

    for pair in raw.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_k, raw_v) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(raw_k);
        let val = percent_decode(raw_v);

        match key.as_str() {
            "name" => out.name = Some(val),
            "tag" | "tag[]" if !val.is_empty() => {
                if out.tags.len() >= MAX_SEARCH_THREADS_TAGS {
                    return Err(AppError::validation(
                        "tag",
                        ValidationCode::InvalidFormat,
                        "Too many tags specified.",
                    ));
                }
                out.tags.push(val);
            }
            "tag_setting" => out.tag_setting = Some(val),
            "archived" => {
                out.archived = match val.as_str() {
                    "true" | "1" => Some(true),
                    "false" | "0" => Some(false),
                    _ => {
                        return Err(AppError::validation(
                            "archived",
                            ValidationCode::InvalidFormat,
                            "archived must be a boolean.",
                        ));
                    }
                };
            }
            "sort_by" => out.sort_by = Some(val),
            "sort_order" => out.sort_order = Some(val),
            "limit" => {
                let n = val.parse::<u32>().map_err(|_| {
                    AppError::validation(
                        "limit",
                        ValidationCode::ValueMustBeIntegerInRange,
                        "limit must be between 1 and 25.",
                    )
                })?;
                if !(1..=MAX_SEARCH_THREADS_LIMIT).contains(&n) {
                    return Err(AppError::validation(
                        "limit",
                        ValidationCode::ValueMustBeIntegerInRange,
                        "limit must be between 1 and 25.",
                    ));
                }
                out.limit = Some(n);
            }
            "offset" => {
                let n = val.parse::<u32>().map_err(|_| {
                    AppError::validation(
                        "offset",
                        ValidationCode::ValueMustBeIntegerInRange,
                        "offset must be between 0 and 9975.",
                    )
                })?;
                if n > MAX_SEARCH_THREADS_OFFSET {
                    return Err(AppError::validation(
                        "offset",
                        ValidationCode::ValueMustBeIntegerInRange,
                        "offset must be between 0 and 9975.",
                    ));
                }
                out.offset = Some(n);
            }
            "max_id" => out.max_id = (!val.is_empty()).then_some(val),
            "min_id" => out.min_id = (!val.is_empty()).then_some(val),
            _ => {}
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_repeated_tags_and_search_params() {
        let q = parse_search_threads_query(Some(
            "name=hello%20world&tag=101&tag=102&archived=false&sort_by=creation_time&sort_order=asc&limit=10&offset=25",
        ))
        .unwrap();
        assert_eq!(q.name.as_deref(), Some("hello world"));
        assert_eq!(q.tags, vec!["101", "102"]);
        assert_eq!(q.archived, Some(false));
        assert_eq!(q.sort_by.as_deref(), Some("creation_time"));
        assert_eq!(q.sort_order.as_deref(), Some("asc"));
        assert_eq!(q.limit, Some(10));
        assert_eq!(q.offset, Some(25));
    }
}
