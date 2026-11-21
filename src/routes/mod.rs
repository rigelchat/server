pub mod api;
pub mod well_known_spacebar;

use axum::{Router, response::Html, routing::get};

use crate::AppState;
use crate::constants::API_VERSION;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .nest("/api", api::router())
        .nest(&format!("/api/v{}", API_VERSION), api::router())
        .nest("/.well-known/spacebar/client", well_known_spacebar::router())
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../../static/index.html"))
}