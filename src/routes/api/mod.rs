mod auth;
mod channels;
mod discoverable_guilds;
mod discovery;
mod gateway;
mod guilds;
mod invites;
mod ping;
mod users;
mod well_known_fluxer;

use axum::Router;

use crate::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .nest("/auth", auth::router())
        .nest("/channels", channels::router())
        .nest("/discoverable-guilds", discoverable_guilds::router())
        .nest("/discovery", discovery::router())
        .nest("/gateway", gateway::router())
        .nest("/guilds", guilds::router())
        .nest("/ping", ping::router())
        .nest("/users", users::router())
        .nest("/invites", invites::router())
        .nest("/.well-known/fluxer", well_known_fluxer::router())
}
