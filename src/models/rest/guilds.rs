use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CreateGuildRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ListGuildsQuery {
    pub before: Option<String>,
    pub after: Option<String>,
    pub limit: Option<u32>,
    pub with_counts: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct ListMembersQuery {
    pub limit: Option<u32>,
    pub after: Option<String>,
}
