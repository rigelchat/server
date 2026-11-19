pub mod handlers;
pub mod session;

use axum::{
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};
use uuid::Uuid;
use zstd::stream::raw::{Decoder as RawZstdDecoder, Operation};

use crate::AppState;
use crate::constants::dialect::ClientDialect;
use crate::constants::gateway::{HEARTBEAT_INTERVAL, MAX_PAYLOAD_BYTES, Opcode, close_codes};
use crate::gateway::handlers::presence::PresenceUpdatePayload;
use crate::gateway::session::GatewaySession;
use crate::models::gateway::{
    GatewayPayload, HelloPayload, IdentifyPayload, lazy_request::LazyRequestPayload,
};

#[derive(Debug, Deserialize)]
pub struct GatewayQuery {
    pub v: Option<String>,
}

pub async fn handler(
    ws: WebSocketUpgrade,
    Query(query): Query<GatewayQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let version = query.v.as_deref().and_then(|v| v.parse::<u8>().ok());
    let dialect = ClientDialect::from_gateway_version(version);
    ws.on_upgrade(move |socket| handle_socket(socket, state, dialect))
}

async fn handle_socket(ws: WebSocket, state: AppState, dialect: ClientDialect) {
    debug!("New WebSocket connection established");

    let (mut socket_sender, mut socket_receiver) = ws.split();
    let session_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::channel::<Message>(100);

    {
        let mut sessions = state.sessions.write().await;
        sessions.insert(
            session_id.clone(),
            GatewaySession::new(session_id.clone(), dialect, tx),
        );
    }

    let hello = GatewayPayload::new(
        Opcode::Hello,
        Some(json!(HelloPayload {
            heartbeat_interval: HEARTBEAT_INTERVAL
        })),
    );

    if let Ok(text) = serde_json::to_string(&hello) {
        let _ = socket_sender.send(Message::Text(text.into())).await;
    }

    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let is_close = matches!(msg, Message::Close(_));
            if socket_sender.send(msg).await.is_err() || is_close {
                break;
            }
        }
    });

    let recv_session_id = session_id.clone();
    let recv_state = state.clone();

    let mut recv_task = tokio::spawn(async move {
        let mut zstd_decoder = RawZstdDecoder::new()
            .map_err(|err| error!(error = %err, "Failed to initialize RawZstdDecoder"))
            .ok();

        while let Some(msg_res) = socket_receiver.next().await {
            let msg = match msg_res {
                Ok(m) => m,
                Err(err) => {
                    error!(error = %err, "WebSocket receive error");
                    break;
                }
            };

            match msg {
                Message::Text(text) => {
                    process_gateway_message(&text, &recv_session_id, &recv_state).await;
                }
                Message::Binary(bin) => {
                    if let Some(text) = decode_binary_frame(&bin, zstd_decoder.as_mut()) {
                        process_gateway_message(&text, &recv_session_id, &recv_state).await;
                    }
                }
                Message::Close(frame) => {
                    debug!(?frame, "Client sent close frame");
                    break;
                }
                _ => {}
            }
        }
    });

    tokio::select! {
        _ = (&mut send_task) => recv_task.abort(),
        _ = (&mut recv_task) => send_task.abort(),
    }

    let (removed, has_other_session, remaining) = {
        let mut sessions = state.sessions.write().await;
        let removed = sessions.remove(&session_id);
        let has_other = removed
            .as_ref()
            .and_then(|r| r.user_id.as_ref())
            .is_some_and(|uid| sessions.values().any(|s| s.user_id.as_deref() == Some(uid)));
        (removed, has_other, sessions.len())
    };

    if let Some(session) = removed {
        if let Some(uid) = session.user_id {
            if has_other_session {
                handlers::presence::dispatch_sessions_replace(&state, &uid).await;
                let pub_status = handlers::presence::user_public_status(&state, &uid).await;
                let custom_status =
                    handlers::presence::fetch_user_custom_status(&state, &uid).await;
                handlers::presence::broadcast_presence(
                    &state,
                    &uid,
                    &session.guilds,
                    pub_status,
                    custom_status.as_ref(),
                )
                .await;
            } else {
                handlers::presence::broadcast_presence(
                    &state,
                    &uid,
                    &session.guilds,
                    "offline",
                    None,
                )
                .await;
            }
            for gid in &session.guilds {
                handlers::lazy_request::refresh_guild_member_lists(&state, gid).await;
            }
        }
    }

    debug!(session_id = %session_id, active_sessions = remaining, "WebSocket session destroyed");
}

fn decode_binary_frame(bin: &[u8], decoder: Option<&mut RawZstdDecoder>) -> Option<String> {
    if let Some(decoder) = decoder {
        let mut out = Vec::new();
        let mut temp = [0u8; 4096];
        let mut src = bin;

        loop {
            match decoder.run_on_buffers(src, &mut temp) {
                Ok(status) => {
                    out.extend_from_slice(&temp[..status.bytes_written]);
                    if out.len() > MAX_PAYLOAD_BYTES {
                        error!("Decompressed gateway frame exceeds max payload limit");
                        return None;
                    }
                    src = &src[status.bytes_read..];
                    if src.is_empty() || (status.bytes_read == 0 && status.bytes_written == 0) {
                        if let Ok(text) = String::from_utf8(out) {
                            return Some(text);
                        }
                        error!("Decompressed gateway frame is not valid UTF-8");
                        return None;
                    }
                }
                Err(err) => {
                    error!(error = ?err, "Zstd streaming decode error");
                    break;
                }
            }
        }
    }

    std::str::from_utf8(bin).ok().map(str::to_string)
}

async fn close_with(session_id: &str, state: &AppState, code: (u16, &'static str)) {
    let sessions = state.sessions.read().await;
    if let Some(session) = sessions.get(session_id) {
        session.close(code).await;
    }
}

async fn process_gateway_message(text: &str, session_id: &str, state: &AppState) {
    debug!(payload = %text, "Received gateway message");

    let payload = match serde_json::from_str::<GatewayPayload>(text) {
        Ok(p) => p,
        Err(err) => {
            error!(error = %err, raw = %text, "Failed to parse GatewayPayload");
            close_with(session_id, state, close_codes::DECODE_ERROR).await;
            return;
        }
    };

    if !matches!(payload.op, Opcode::Identify | Opcode::Heartbeat) {
        let authenticated = {
            let sessions = state.sessions.read().await;
            sessions
                .get(session_id)
                .is_some_and(|s| s.user_id.is_some())
        };
        if !authenticated {
            close_with(session_id, state, close_codes::NOT_AUTHENTICATED).await;
            return;
        }
    }

    match payload.op {
        Opcode::Identify => {
            let Some(d) = payload.d else {
                close_with(session_id, state, close_codes::DECODE_ERROR).await;
                return;
            };
            match serde_json::from_value::<IdentifyPayload>(d) {
                Ok(identify) => handlers::identify::handle(session_id, state, identify).await,
                Err(err) => {
                    error!(error = %err, "Failed to parse IDENTIFY payload");
                    close_with(session_id, state, close_codes::DECODE_ERROR).await;
                }
            }
        }
        Opcode::Heartbeat => handlers::heartbeat::handle(session_id, state).await,
        Opcode::PresenceUpdate => {
            let Some(d) = payload.d else {
                close_with(session_id, state, close_codes::DECODE_ERROR).await;
                return;
            };
            match serde_json::from_value::<PresenceUpdatePayload>(d) {
                Ok(presence_req) => handlers::presence::handle(session_id, state, presence_req).await,
                Err(err) => {
                    error!(error = %err, "Failed to parse PRESENCE_UPDATE payload");
                    close_with(session_id, state, close_codes::DECODE_ERROR).await;
                }
            }
        }
        Opcode::LazyRequest => {
            let Some(d) = payload.d else {
                return;
            };
            match serde_json::from_value::<LazyRequestPayload>(d) {
                Ok(lazy_req) => handlers::lazy_request::handle(session_id, state, lazy_req).await,
                Err(err) => error!(error = %err, "Failed to parse LAZY_REQUEST payload"),
            }
        }
        other => {
            warn!(opcode = ?other, "Unhandled gateway opcode");
        }
    }
}