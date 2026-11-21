use serde::Serialize;

/// See: `https://github.com/spacebarchat/server/tree/master/src/schemas/responses/InstancePingResponse.ts`
#[derive(Serialize)]
pub struct PingResponse {
    pub ping: String,
    pub instance: Instance,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Instance {
    pub name: String,
    pub description: Option<String>,
    pub image: Option<String>,
    pub correspondence_email: Option<String>,
    #[serde(rename = "correspondenceUserID")]
    pub correspondence_user_id: Option<String>,
    pub front_page: Option<String>,
    pub tos_page: Option<String>,
}
