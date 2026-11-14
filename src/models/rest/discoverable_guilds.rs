use serde::Serialize;

#[derive(Serialize)]
pub struct DiscoverableGuildsResponse {
    pub guilds: Vec<DiscoverableGuild>,
    pub offset: u64,
    pub limit: u64,
    pub total: u64
}

#[derive(Serialize)]
pub struct DiscoverableGuild {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub splash: Option<String>,
    pub discovery_splash: Option<String>,
    pub features: Vec<String>,
    pub approximate_presence_count: u64,
    pub approximate_member_count: u64,
    pub premium_subscription_count: u64
}