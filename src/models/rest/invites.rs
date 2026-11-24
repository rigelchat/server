use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct CreateInviteRequest {
    pub max_uses: Option<u32>,
    pub max_age: Option<u32>,
    pub temporary: Option<bool>,
}
