use serde::{Deserialize, Serialize};

use crate::models::read_state::ReadState;

#[derive(Debug, Deserialize)]
pub struct ReadStateAckEntryInput {
    pub channel_id: String,
    pub message_id: String,
    pub mention_count: Option<i64>,
    pub manual: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct BulkReadStateAckEntryInput {
    pub channel_id: String,
    pub message_id: String,
}

#[derive(Debug, Deserialize)]
pub struct AckReadStatesRequest {
    pub read_states: Vec<ReadStateAckEntryInput>,
}

#[derive(Debug, Serialize)]
pub struct AckReadStatesResponse {
    pub read_states: Vec<ReadState>,
}

#[derive(Debug, Deserialize)]
pub struct BulkAckReadStatesRequest {
    pub read_states: Vec<BulkReadStateAckEntryInput>,
}

#[derive(Debug, Default, Deserialize)]
pub struct AckMessageRequest {
    pub manual: Option<bool>,
    pub mention_count: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AckMessageResponse {
    pub token: Option<String>,
}
