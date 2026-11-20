pub mod handlers;
pub mod hub;
pub mod session;

use std::sync::Arc;
use std::time::Duration;

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
    let (tx, mut rx) = mpsc::channel::<Message>(512);

    let session = Arc::new(GatewaySession::new(session_id.clone(), dialect, tx));
    state.hub.register(Arc::clone(&session));

    session.send(
        Opcode::Hello,
        Some(json!(HelloPayload {
            heartbeat_interval: HEARTBEAT_INTERVAL
        })),
    );

    let mut send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let is_close = matches!(msg, Message::Close(_));
            match tokio::time::timeout(Duration::from_secs(5), socket_sender.send(msg)).await {
                Ok(Ok(())) if !is_close => {}
                _ => break,
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
                    if text.len() > MAX_PAYLOAD_BYTES {
                        error!("Text gateway frame exceeds max payload limit");
                        recv_state
                            .hub
                            .close_session(&recv_session_id, close_codes::DECODE_ERROR);
                        break;
                    }
                    process_gateway_message(&text, &recv_session_id, &recv_state).await;
                }
                Message::Binary(bin) => {
                    match decode_binary_frame(&bin, zstd_decoder.as_mut()) {
                        Ok(text) => {
                            process_gateway_message(&text, &recv_session_id, &recv_state).await
                        }
                        Err(code) => {
                            recv_state.hub.close_session(&recv_session_id, code);
                            break;
                        }
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
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
        _ = session.wait_for_close() => {
            send_task.abort();
            recv_task.abort();
        }
    }

    let (removed, remaining) = state.hub.unregister(&session_id);

    if let Some(removed) = removed {
        if removed.has_other_session {
            handlers::presence::dispatch_sessions_replace(&state, &removed.user_id).await;
            let current_live = handlers::presence::collect_user_live_state(&state, &removed.user_id).await;
            if current_live != removed.prev_live_state {
                handlers::presence::broadcast_presence_with_user(
                    &state,
                    &removed.user_id,
                    &removed.guilds,
                    current_live.0,
                    removed.custom_status.as_ref(),
                    removed.partial_user.as_ref(),
                )
                .await;
                for gid in &removed.guilds {
                    handlers::lazy_request::refresh_guild_member_lists(&state, gid).await;
                }
            }
        } else {
            handlers::presence::broadcast_presence_with_user(
                &state,
                &removed.user_id,
                &removed.guilds,
                "offline",
                None,
                removed.partial_user.as_ref(),
            )
            .await;
            for gid in &removed.guilds {
                handlers::lazy_request::refresh_guild_member_lists(&state, gid).await;
            }
        }
    }

    debug!(session_id = %session_id, active_sessions = remaining, "WebSocket session destroyed");
}

fn decode_binary_frame(
    bin: &[u8],
    decoder: Option<&mut RawZstdDecoder>,
) -> Result<String, (u16, &'static str)> {
    if bin.len() > MAX_PAYLOAD_BYTES {
        error!("Compressed gateway frame exceeds max payload limit");
        return Err(close_codes::DECODE_ERROR);
    }

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
                        return Err(close_codes::DECODE_ERROR);
                    }
                    src = &src[status.bytes_read..];
                    if src.is_empty() && status.bytes_written < temp.len() {
                        return String::from_utf8(out).map_err(|_| {
                            error!("Decompressed gateway frame is not valid UTF-8");
                            close_codes::DECODE_ERROR
                        });
                    }
                    if status.bytes_read == 0 && status.bytes_written == 0 {
                        error!("Zstd decoder stalled on gateway frame");
                        return Err(close_codes::DECODE_ERROR);
                    }
                }
                Err(err) => {
                    error!(error = ?err, "Zstd streaming decode error");
                    return Err(close_codes::DECODE_ERROR);
                }
            }
        }
    }

    std::str::from_utf8(bin)
        .map(str::to_string)
        .map_err(|_| close_codes::DECODE_ERROR)
}

async fn process_gateway_message(text: &str, session_id: &str, state: &AppState) {
    debug!(payload = %text, "Received gateway message");

    let payload = match serde_json::from_str::<GatewayPayload>(text) {
        Ok(p) => p,
        Err(err) => {
            error!(error = %err, raw = %text, "Failed to parse GatewayPayload");
            state.hub.close_session(session_id, close_codes::DECODE_ERROR);
            return;
        }
    };

    if !matches!(payload.op, Opcode::Identify | Opcode::Heartbeat) {
        let authenticated = state
            .hub
            .get(session_id)
            .is_some_and(|s| s.is_authenticated());
        if !authenticated {
            state
                .hub
                .close_session(session_id, close_codes::NOT_AUTHENTICATED);
            return;
        }
    }

    match payload.op {
        Opcode::Identify => {
            let Some(d) = payload.d else {
                state.hub.close_session(session_id, close_codes::DECODE_ERROR);
                return;
            };
            match serde_json::from_value::<IdentifyPayload>(d) {
                Ok(identify) => handlers::identify::handle(session_id, state, identify).await,
                Err(err) => {
                    error!(error = %err, "Failed to parse IDENTIFY payload");
                    state.hub.close_session(session_id, close_codes::DECODE_ERROR);
                }
            }
        }
        Opcode::Heartbeat => handlers::heartbeat::handle(session_id, state).await,
        Opcode::PresenceUpdate => {
            let Some(d) = payload.d else {
                state.hub.close_session(session_id, close_codes::DECODE_ERROR);
                return;
            };
            match serde_json::from_value::<PresenceUpdatePayload>(d) {
                Ok(presence_req) => handlers::presence::handle(session_id, state, presence_req).await,
                Err(err) => {
                    error!(error = %err, "Failed to parse PRESENCE_UPDATE payload");
                    state.hub.close_session(session_id, close_codes::DECODE_ERROR);
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