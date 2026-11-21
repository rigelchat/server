pub mod events;
pub mod lazy_request;
pub mod presence;
pub mod ready;

use serde::{Deserialize, Serialize};

use crate::constants::gateway::Opcode;

#[derive(Debug, Deserialize)]
pub struct GatewayQuery {
    pub compress: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct GatewayPayload<T = serde_json::Value> {
    pub op: Opcode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
}

impl<T> GatewayPayload<T> {
    pub fn new(op: Opcode, d: Option<T>) -> Self {
        Self {
            op,
            d,
            s: None,
            t: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HelloPayload {
    pub heartbeat_interval: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentifyPresence {
    pub status: Option<String>,
    pub afk: Option<bool>,
    pub mobile: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IdentifyPayload {
    pub token: String,
    pub properties: serde_json::Value,
    pub presence: Option<IdentifyPresence>,
}
