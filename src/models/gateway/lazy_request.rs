use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::models::guild_member::GuildMemberFields;
use crate::models::user::{CustomStatus, PartialUser};

#[derive(Debug, Deserialize)]
pub struct LazyRequestPayload {
    pub subscriptions: Option<HashMap<String, GuildSubscription>>,
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
pub struct MemberListPresence {
    pub status: String,
    pub mobile: bool,
    pub afk: bool,
    pub custom_status: Option<CustomStatus>,
}

#[derive(Clone, Serialize)]
pub struct MemberListMember {
    pub user: PartialUser,
    #[serde(flatten)]
    pub member: GuildMemberFields,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fluxer_lazy_requests() {
        let fluxer_json = r#"{
            "subscriptions": {
                "g1": {
                    "member_list_channels": { "c1": [[0, 99]] },
                    "members": ["u1", "u2"]
                },
                "g2": {
                    "member_list_channels": {}
                }
            }
        }"#;
        let payload: LazyRequestPayload = serde_json::from_str(fluxer_json).unwrap();
        assert_eq!(payload.into_actions().len(), 3);

        let unsub_json = r#"{
            "subscriptions": {
                "g1": { "member_list_channels": {} }
            }
        }"#;
        let unsub: LazyRequestPayload = serde_json::from_str(unsub_json).unwrap();
        assert!(matches!(
            &unsub.into_actions()[..],
            [LazyListAction::Unsubscribe { guild_id }] if guild_id == "g1"
        ));
    }
}