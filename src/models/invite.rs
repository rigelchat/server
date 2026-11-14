#![allow(dead_code)]

use serde::{Serialize, Deserialize};

/// See: https://docs.discord.com/developers/resources/invite#invite-object-invite-types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InviteType {
    Guild = 0,
    GroupDm = 1,
    Friend = 2
}