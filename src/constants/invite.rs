#![allow(dead_code)]

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(into = "u8")]
pub enum InviteType {
    Guild = 0,
    GroupDm = 1,
    Friend = 2,
}

impl From<InviteType> for u8 {
    fn from(kind: InviteType) -> u8 {
        kind as u8
    }
}
