pub mod api;
pub mod well_known_spacebar;

use axum::{middleware, Router, routing::get, response::Html};

use crate::AppState;
use crate::constants::API_VERSION;
use crate::error::adapt_error_dialect;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(index))
        .nest("/api", api::router())
        .nest(&format!("/api/v{}", API_VERSION), api::router()) 
        .nest("/.well-known/spacebar/client", well_known_spacebar::router())
        .layer(middleware::from_fn(adapt_error_dialect))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../../static/index.html"))
}