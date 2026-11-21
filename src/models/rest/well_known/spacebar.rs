#![allow(dead_code)]

use serde::Serialize;

use crate::constants::gateway::{GatewayCompression, GatewayEncoding};

/// See: `https://github.com/spacebarchat/server/tree/master/src/schemas/api/spacebar/WellKnown.ts`
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigResponse {
    pub api: Api,
    pub cdn: Cdn,
    pub gateway: Gateway,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Api {
    pub base_url: String,
    pub api_versions: ApiVersions,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiVersions {
    pub default: u8,
    pub active: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cdn {
    pub base_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gateway {
    pub base_url: String,
    pub encoding: Vec<GatewayEncoding>,
    pub compression: Vec<Option<GatewayCompression>>,
}
