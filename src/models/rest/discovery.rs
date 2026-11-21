use serde::{Deserialize, Serialize};

use crate::constants::guild::{DiscoveryCategory, VerificationLevel};

#[derive(Debug, Deserialize)]
pub struct DiscoverySearchQuery {
    pub query: Option<String>,
    pub category: Option<u8>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

#[derive(Serialize)]
pub struct GuildDiscoveryResponse {
    pub guilds: Vec<DiscoverableGuild>,
    pub total: u64,
    pub category_counts: Vec<CategoryCount>,
}

#[derive(Serialize)]
pub struct DiscoverableGuild {
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub description: Option<String>,
    pub category_type: DiscoveryCategory,
    pub primary_language: String,
    pub custom_tags: Vec<String>,
    pub member_count: u64,
    pub online_count: u64,
    pub features: Vec<String>,
    pub badge_ids: Vec<String>,
    pub verification_level: VerificationLevel,
}

#[derive(Serialize)]
pub struct CategoryCount {
    pub category_type: DiscoveryCategory,
    pub count: u64,
}

#[derive(Serialize)]
pub struct DiscoveryCategoryResponse {
    pub id: u8,
    pub name: String,
}
