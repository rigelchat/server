use axum::{Json, Router, extract::State, routing::get};

use crate::AppState;
use crate::models::rest::ping::{Instance, PingResponse};

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_instance_info))
}

async fn get_instance_info(State(state): State<AppState>) -> Json<PingResponse> {
    Json(PingResponse {
        ping: "pong!".to_string(),
        instance: Instance {
            name: state.config.instance.name.clone(),
            description: state.config.instance.description.clone(),
            image: state.config.instance.image.clone(),
            correspondence_email: state.config.instance.contact_email.clone(),
            correspondence_user_id: state.config.instance.owner_id.clone(),
            front_page: state.config.instance.website_url.clone(),
            tos_page: state.config.instance.terms_url.clone(),
        },
    })
}
