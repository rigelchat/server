use serde::{Serialize, Deserialize};

/// See: https://docs.discord.com/developers/resources/channel#overwrite-object
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverwriteType {
    Role = 0,
    Member = 1
}