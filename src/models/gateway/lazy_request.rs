use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::guild_member::GuildMemberUserRef;
use crate::models::serde_helpers::serialize_millis_iso;
use crate::models::user::{CustomStatus, PartialUser};

#[derive(Debug, Deserialize)]
pub struct LazyRequestPayload {
    pub subscriptions: Option<HashMap<String, GuildSubscription>>,
    pub guild_id: Option<String>,
    pub channels: Option<HashMap<String, Vec<Vec<i64>>>>,
}

#[derive(Debug, Deserialize)]
pub struct GuildSubscription {
    pub member_list_channels: Option<HashMap<String, Vec<Vec<i64>>>>,
}

impl LazyRequestPayload {
    pub fn extract(self) -> Option<(String, String, [i64; 2])> {
        let mut range = [0i64, 99i64];

        if let Some(subs) = self.subscriptions {
            let (guild_id, sub) = subs.into_iter().next()?;
            let mut channel_id = "everyone".to_string();

            if let Some(channels) = sub.member_list_channels {
                if let Some((cid, ranges)) = channels.into_iter().next() {
                    channel_id = cid;
                    if let Some(first) = ranges.first() {
                        if first.len() >= 2 {
                            range = [first[0], first[1]];
                        }
                    }
                }
            }
            return Some((guild_id, channel_id, range));
        }

        let guild_id = self.guild_id?;
        let mut channel_id = "everyone".to_string();

        if let Some(channels) = self.channels {
            if let Some((cid, ranges)) = channels.into_iter().next() {
                channel_id = cid;
                if let Some(first) = ranges.first() {
                    if first.len() >= 2 {
                        range = [first[0], first[1]];
                    }
                }
            }
        }

        Some((guild_id, channel_id, range))
    }
}

#[derive(Clone, Serialize)]
pub struct MemberListGroup {
    pub id: String,
    pub count: usize,
}

#[derive(Clone, Serialize)]
pub struct MemberListClientStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub desktop: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum MemberListPresence {
    Fluxer {
        status: String,
        mobile: bool,
        afk: bool,
        custom_status: Option<CustomStatus>,
    },
    Discord {
        user: GuildMemberUserRef,
        status: String,
        client_status: MemberListClientStatus,
        activities: Vec<serde_json::Value>,
    },
}

#[derive(Clone, Serialize)]
pub struct MemberListMember {
    pub user: PartialUser,
    pub nick: Option<String>,
    pub avatar: Option<String>,
    pub banner: Option<String>,
    pub accent_color: Option<u32>,
    pub roles: Vec<String>,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
    pub mute: bool,
    pub deaf: bool,
    pub communication_disabled_until: Option<String>,
    pub presence: MemberListPresence,
}

#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum MemberListItem {
    Group { group: MemberListGroup },
    Member { member: MemberListMember },
}

#[derive(Serialize)]
pub struct MemberListSyncOp {
    pub op: &'static str,
    pub range: [i64; 2],
    pub items: Vec<MemberListItem>,
}

#[derive(Serialize)]
pub struct GuildMemberListUpdate {
    pub guild_id: String,
    pub channel_id: String,
    pub id: String,
    pub ops: Vec<MemberListSyncOp>,
    pub online_count: usize,
    pub member_count: usize,
    pub groups: Vec<MemberListGroup>,
}