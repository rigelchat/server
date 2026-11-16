use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::guild_member::GuildMemberUserRef;
use crate::models::serde_helpers::serialize_millis_iso;

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
    pub desktop: String,
}

#[derive(Clone, Serialize)]
pub struct MemberListPresence {
    pub user: GuildMemberUserRef,
    pub status: String,
    pub client_status: MemberListClientStatus,
    pub activities: Vec<serde_json::Value>,
}

#[derive(Clone, Serialize)]
pub struct MemberListUser {
    pub id: String,
    pub username: String,
    pub discriminator: String,
    pub global_name: Option<String>,
    pub avatar: Option<String>,
    pub bot: bool,
}

#[derive(Clone, Serialize)]
pub struct MemberListMember {
    pub user: MemberListUser,
    pub nick: Option<String>,
    pub roles: Vec<String>,
    pub mute: bool,
    pub deaf: bool,
    #[serde(serialize_with = "serialize_millis_iso")]
    pub joined_at: u64,
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