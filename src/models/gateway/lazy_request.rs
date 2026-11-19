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
    pub members: Option<Vec<String>>,
}

pub enum LazyListAction {
    Subscribe {
        guild_id: String,
        channel_id: String,
        range: [i64; 2],
    },
    Unsubscribe {
        guild_id: String,
    },
    SubscribeMembers {
        guild_id: String,
        user_ids: Vec<String>,
    },
}

impl LazyRequestPayload {
    pub fn into_actions(self) -> Vec<LazyListAction> {
        let mut actions = Vec::new();

        if let Some(subs) = self.subscriptions {
            for (guild_id, sub) in subs {
                if let Some(user_ids) = sub.members {
                    if !user_ids.is_empty() {
                        actions.push(LazyListAction::SubscribeMembers {
                            guild_id: guild_id.clone(),
                            user_ids,
                        });
                    }
                }
                if let Some(channels) = sub.member_list_channels {
                    if channels.is_empty() {
                        actions.push(LazyListAction::Unsubscribe { guild_id });
                        continue;
                    }
                    for (channel_id, ranges) in channels {
                        if let Some(first) = ranges.first().filter(|r| r.len() >= 2) {
                            actions.push(LazyListAction::Subscribe {
                                guild_id: guild_id.clone(),
                                channel_id,
                                range: [first[0], first[1]],
                            });
                        } else {
                            actions.push(LazyListAction::Unsubscribe {
                                guild_id: guild_id.clone(),
                            });
                        }
                    }
                }
            }
            return actions;
        }

        if let Some(guild_id) = self.guild_id {
            if let Some(channels) = self.channels {
                for (channel_id, ranges) in channels {
                    if let Some(first) = ranges.first().filter(|r| r.len() >= 2) {
                        actions.push(LazyListAction::Subscribe {
                            guild_id: guild_id.clone(),
                            channel_id,
                            range: [first[0], first[1]],
                        });
                    }
                }
            }
        }

        actions
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