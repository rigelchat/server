use axum::{
    Json, Router,
    extract::{Path, Query, RawQuery, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, patch, post, put},
};

use crate::AppState;
use crate::error::AppError;
use crate::extractors::{AppJson, AuthUser, FluxerFeatures};
use crate::models::rest::threads::{
    GetThreadMemberQuery, ListArchivedThreadsQuery, ListThreadMembersQuery,
    ModifyThreadMemberSettingsRequest, SearchThreadsResponse, StartThreadFromMessageRequest,
    StartThreadRequest, ThreadListResponse, parse_search_threads_query,
};
use crate::models::thread::{StartThreadResponse, ThreadMember, ThreadResponse};
use crate::services::thread;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/{id}/threads", post(start_thread))
        .route(
            "/{id}/messages/{message_id}/threads",
            post(start_thread_from_message),
        )
        .route(
            "/{id}/threads/archived/public",
            get(list_public_archived_threads),
        )
        .route(
            "/{id}/threads/archived/private",
            get(list_private_archived_threads),
        )
        .route(
            "/{id}/users/@me/threads/archived/private",
            get(list_joined_private_archived_threads),
        )
        .route("/{id}/threads/search", get(search_threads))
        .route(
            "/{id}/thread-members/@me",
            put(join_thread).delete(leave_thread),
        )
        .route(
            "/{id}/thread-members/@me/settings",
            patch(modify_thread_member_settings),
        )
        .route("/{id}/thread-members", get(list_thread_members))
        .route(
            "/{id}/thread-members/{user_id}",
            get(get_thread_member)
                .put(add_thread_member)
                .delete(remove_thread_member),
        )
}

async fn start_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
    AppJson(payload): AppJson<StartThreadRequest>,
) -> Result<(StatusCode, Json<StartThreadResponse>), AppError> {
    let res = thread::start_thread(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        auth.is_bot,
        payload,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(res)))
}

async fn start_thread_from_message(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path((channel_id, message_id)): Path<(String, String)>,
    AppJson(payload): AppJson<StartThreadFromMessageRequest>,
) -> Result<(StatusCode, Json<ThreadResponse>), AppError> {
    let res = thread::start_thread_from_message(
        &state,
        &channel_id,
        &message_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        auth.is_bot,
        payload,
    )
    .await?;
    Ok((StatusCode::CREATED, Json(res)))
}

async fn list_public_archived_threads(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
    Query(query): Query<ListArchivedThreadsQuery>,
) -> Result<Json<ThreadListResponse>, AppError> {
    let res = thread::list_public_archived_threads(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        query,
    )
    .await?;
    Ok(Json(res))
}

async fn list_private_archived_threads(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
    Query(query): Query<ListArchivedThreadsQuery>,
) -> Result<Json<ThreadListResponse>, AppError> {
    let res = thread::list_private_archived_threads(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        query,
    )
    .await?;
    Ok(Json(res))
}

async fn list_joined_private_archived_threads(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
    Query(query): Query<ListArchivedThreadsQuery>,
) -> Result<Json<ThreadListResponse>, AppError> {
    let res = thread::list_joined_private_archived_threads(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        query,
    )
    .await?;
    Ok(Json(res))
}

async fn search_threads(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
    RawQuery(raw): RawQuery,
) -> Result<Json<SearchThreadsResponse>, AppError> {
    let query = parse_search_threads_query(raw.as_deref())?;
    let res = thread::search_threads(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
        query,
    )
    .await?;
    Ok(Json(res))
}

async fn join_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
) -> Result<StatusCode, AppError> {
    thread::join_thread(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn leave_thread(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path(channel_id): Path<String>,
) -> Result<StatusCode, AppError> {
    thread::leave_thread(
        &state,
        &channel_id,
        &auth.user_id,
        features.has_channel_threads(auth.is_bot),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_thread_member(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path((channel_id, target_user_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    thread::add_thread_member(
        &state,
        &channel_id,
        &auth.user_id,
        &target_user_id,
        features.has_channel_threads(auth.is_bot),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_thread_member(
    State(state): State<AppState>,
    auth: AuthUser,
    features: FluxerFeatures,
    Path((channel_id, target_user_id)): Path<(String, String)>,
) -> Result<StatusCode, AppError> {
    thread::remove_thread_member(
        &state,
        &channel_id,
        &auth.user_id,
        &target_user_id,
        features.has_channel_threads(auth.is_bot),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_thread_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((channel_id, target_user_id)): Path<(String, String)>,
    Query(query): Query<GetThreadMemberQuery>,
) -> Result<Json<ThreadMember>, AppError> {
    let member = thread::get_thread_member(
        &state,
        &channel_id,
        &auth.user_id,
        &target_user_id,
        auth.is_bot,
        query.with_member.unwrap_or(false),
    )
    .await?;
    Ok(Json(member))
}

async fn list_thread_members(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
    Query(query): Query<ListThreadMembersQuery>,
) -> Result<Json<Vec<ThreadMember>>, AppError> {
    let members =
        thread::list_thread_members(&state, &channel_id, &auth.user_id, auth.is_bot, query).await?;
    Ok(Json(members))
}

async fn modify_thread_member_settings(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(channel_id): Path<String>,
    AppJson(payload): AppJson<ModifyThreadMemberSettingsRequest>,
) -> Result<Response, AppError> {
    let updated = thread::modify_thread_member_settings(
        &state,
        &channel_id,
        &auth.user_id,
        auth.is_bot,
        payload,
    )
    .await?;
    Ok(match updated {
        Some(member) => Json(member).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    })
}
