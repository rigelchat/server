use serde::Serialize;

#[derive(Serialize)]
pub struct GatewayResponse {
    pub url: String,
}

#[derive(Serialize)]
pub struct GatewayBotResponse {
    pub url: String,
    pub shards: u8,
    pub session_start_limit: SessionStartLimit,
}

#[derive(Serialize)]
pub struct SessionStartLimit {
    pub remaining: u32,
    pub total: u32,
    pub max_concurrency: u8,
    pub reset_after: u64,
}