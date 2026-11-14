pub mod handlers;
pub mod session;

use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};
use uuid::Uuid;
use zstd::stream::raw::{Decoder as RawZstdDecoder, Operation};

use crate::AppState;
use crate::constants::gateway::{HEARTBEAT_INTERVAL, Opcode};
use crate::gateway::session::GatewaySession;
use crate::models::gateway::{GatewayPayload, HelloPayload, IdentifyPayload};

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(ws: WebSocket, state: AppState) {
    debug!("New WebSocket connection established");

    let (mut socket_sender, mut socket_receiver) = ws.split();
    let session_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::channel::<Message>(100);

    {
        let mut sessions = state.sessions.write().await;
        sessions.insert(session_id.clone(), GatewaySession::new(session_id.clone(), tx));
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

    let remaining = {
        let mut sessions = state.sessions.write().await;
        sessions.remove(&session_id);
        sessions.len()
    };

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

async fn process_gateway_message(text: &str, session_id: &str, state: &AppState) {
    debug!(payload = %text, "Received gateway message");

    let payload = match serde_json::from_str::<GatewayPayload>(text) {
        Ok(p) => p,
        Err(err) => {
            error!(error = %err, raw = %text, "Failed to parse GatewayPayload");
            return;
        }
    };

    match payload.op {
        Opcode::Identify => {
            let Some(d) = payload.d else {
                error!("IDENTIFY received without 'd' payload");
                return;
            };
            match serde_json::from_value::<IdentifyPayload>(d.clone()) {
                Ok(identify) => handlers::identify::handle(session_id, state, identify).await,
                Err(err) => error!(error = %err, raw = ?d, "Failed to parse IDENTIFY payload"),
            }
        }
        Opcode::Heartbeat => handlers::heartbeat::handle(session_id, state).await,
        Opcode::LazyRequest => handlers::lazy_request::handle(session_id, state, &payload).await,
        other => warn!(opcode = ?other, "Unhandled gateway opcode"),
    }
}