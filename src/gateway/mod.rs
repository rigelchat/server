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
use serde_json::json;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};
use uuid::Uuid;
use zstd::stream::raw::{Decoder as RawZstdDecoder, Operation};

use crate::AppState;
use crate::constants::gateway::{
    GATEWAY_SEND_QUEUE_CAPACITY, GATEWAY_WRITE_TIMEOUT_SECS, HEARTBEAT_INTERVAL, MAX_PAYLOAD_BYTES,
    Opcode, ZSTD_COMPRESSION_LEVEL, ZSTD_DECODE_CHUNK_BYTES, close_codes,
};
use crate::gateway::session::GatewaySession;
use crate::models::gateway::{
    GatewayPayload, GatewayQuery, HelloPayload, IdentifyPayload, lazy_request::LazyRequestPayload,
    presence::PresenceUpdatePayload,
};

pub async fn handler(
    ws: WebSocketUpgrade,
    Query(query): Query<GatewayQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    let zstd_stream = query
        .compress
        .as_deref()
        .is_some_and(|c| c.eq_ignore_ascii_case("zstd-stream"));
    ws.on_upgrade(move |socket| handle_socket(socket, state, zstd_stream))
}

async fn handle_socket(ws: WebSocket, state: AppState, zstd_stream: bool) {
    debug!("New WebSocket connection established");

    let (mut socket_sender, mut socket_receiver) = ws.split();
    let session_id = Uuid::new_v4().to_string();
    let (tx, mut rx) = mpsc::channel::<Message>(GATEWAY_SEND_QUEUE_CAPACITY);

    let session = Arc::new(GatewaySession::new(session_id.clone(), tx));
    state.hub.register(Arc::clone(&session));

    session.send(
        Opcode::Hello,
        Some(json!(HelloPayload {
            heartbeat_interval: HEARTBEAT_INTERVAL
        })),
    );

    let mut send_task = tokio::spawn(async move {
        let mut zstd_encoder = if zstd_stream {
            zstd::stream::write::Encoder::new(Vec::new(), ZSTD_COMPRESSION_LEVEL)
                .map_err(|err| error!(error = %err, "Failed to initialize zstd encoder"))
                .ok()
        } else {
            None
        };

        while let Some(msg) = rx.recv().await {
            let is_close = matches!(msg, Message::Close(_));
            let outgoing = match (msg, zstd_encoder.as_mut()) {
                (Message::Text(text), Some(encoder)) => {
                    use std::io::Write;
                    if let Err(err) = encoder
                        .write_all(text.as_bytes())
                        .and_then(|_| encoder.flush())
                    {
                        error!(error = %err, "Failed to zstd-compress gateway frame");
                        break;
                    }
                    let compressed = std::mem::take(encoder.get_mut());
                    Message::Binary(compressed.into())
                }
                (other, _) => other,
            };
            match tokio::time::timeout(
                Duration::from_secs(GATEWAY_WRITE_TIMEOUT_SECS),
                socket_sender.send(outgoing),
            )
            .await
            {
                Ok(Ok(())) if !is_close => {}
                _ => break,
            }
        }
    });

    let recv_session_id = session_id.clone();
    let recv_state = state.clone();

    let mut recv_task = tokio::spawn(async move {
        let mut zstd_decoder = if zstd_stream {
            RawZstdDecoder::new()
                .map_err(|err| error!(error = %err, "Failed to initialize RawZstdDecoder"))
                .ok()
        } else {
            None
        };

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
                    if recv_state
                        .hub
                        .get(&recv_session_id)
                        .is_none_or(|s| s.is_closed())
                    {
                        break;
                    }
                }
                Message::Binary(bin) => match decode_binary_frame(&bin, zstd_decoder.as_mut()) {
                    Ok(text) => {
                        process_gateway_message(&text, &recv_session_id, &recv_state).await;
                        if recv_state
                            .hub
                            .get(&recv_session_id)
                            .is_none_or(|s| s.is_closed())
                        {
                            break;
                        }
                    }
                    Err(code) => {
                        recv_state.hub.close_session(&recv_session_id, code);
                        break;
                    }
                },
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
        _ = &mut recv_task => {
            if session.is_closed() {
                if tokio::time::timeout(
                    Duration::from_secs(GATEWAY_WRITE_TIMEOUT_SECS),
                    &mut send_task,
                )
                .await
                .is_err()
                {
                    send_task.abort();
                }
            } else {
                send_task.abort();
            }
        }
        _ = session.wait_for_close() => {
            send_task.abort();
            recv_task.abort();
        }
    }

    let (removed, remaining) = state.hub.unregister(&session_id);

    if let Some(removed) = removed {
        if removed.has_other_session {
            handlers::presence::dispatch_sessions_replace(&state, &removed.user_id).await;
            let current_live =
                handlers::presence::collect_user_live_state(&state, &removed.user_id).await;
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
        } else if removed.prev_live_state.0 != "offline" {
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
        let mut temp = [0u8; ZSTD_DECODE_CHUNK_BYTES];
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
                    if !src.is_empty() && status.bytes_read == 0 && status.bytes_written == 0 {
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
    let payload = match serde_json::from_str::<GatewayPayload>(text) {
        Ok(p) => p,
        Err(err) => {
            error!(error = %err, bytes = text.len(), "Failed to parse GatewayPayload");
            state
                .hub
                .close_session(session_id, close_codes::DECODE_ERROR);
            return;
        }
    };

    let Some(session) = state.hub.get(session_id) else {
        return;
    };

    let authenticated = session.is_authenticated();
    let is_expensive = matches!(payload.op, Opcode::LazyRequest | Opcode::PresenceUpdate);
    if !session.check_rate_limit(authenticated, is_expensive) {
        warn!(session_id = %session_id, op = ?payload.op, "Closing rate-limited gateway session");
        session.close(close_codes::RATE_LIMITED);
        return;
    }

    debug!(op = ?payload.op, bytes = text.len(), "Received gateway message");

    if !matches!(payload.op, Opcode::Identify | Opcode::Heartbeat) && !authenticated {
        session.close(close_codes::NOT_AUTHENTICATED);
        return;
    }

    match payload.op {
        Opcode::Identify => {
            let Some(d) = payload.d else {
                session.close(close_codes::DECODE_ERROR);
                return;
            };
            match serde_json::from_value::<IdentifyPayload>(d) {
                Ok(identify) => handlers::identify::handle(session_id, state, identify).await,
                Err(err) => {
                    error!(error = %err, "Failed to parse IDENTIFY payload");
                    session.close(close_codes::DECODE_ERROR);
                }
            }
        }
        Opcode::Heartbeat => handlers::heartbeat::handle(session_id, state).await,
        Opcode::PresenceUpdate => {
            let Some(d) = payload.d else {
                session.close(close_codes::DECODE_ERROR);
                return;
            };
            match serde_json::from_value::<PresenceUpdatePayload>(d) {
                Ok(presence_req) => {
                    handlers::presence::handle(session_id, state, presence_req).await
                }
                Err(err) => {
                    error!(error = %err, "Failed to parse PRESENCE_UPDATE payload");
                    session.close(close_codes::DECODE_ERROR);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_binary_frame_handles_exact_4096_boundary() {
        let prefix = r#"{"op":1,"d":""#;
        let suffix = r#""}"#;
        let filler = "a".repeat(4096 - prefix.len() - suffix.len());
        let payload = format!("{prefix}{filler}{suffix}");
        assert_eq!(payload.len(), 4096);

        let compressed = zstd::stream::encode_all(payload.as_bytes(), 3).unwrap();
        let mut decoder = RawZstdDecoder::new().unwrap();
        let decoded = decode_binary_frame(&compressed, Some(&mut decoder))
            .expect("4096-byte payload should decode cleanly");
        assert_eq!(decoded, payload);
    }

    #[test]
    fn decode_binary_frame_rejects_decompression_bomb() {
        let oversized = vec![b'x'; MAX_PAYLOAD_BYTES + 100];
        let compressed = zstd::stream::encode_all(&oversized[..], 3).unwrap();
        assert!(compressed.len() < MAX_PAYLOAD_BYTES);

        let mut decoder = RawZstdDecoder::new().unwrap();
        let err = decode_binary_frame(&compressed, Some(&mut decoder)).unwrap_err();
        assert_eq!(err, close_codes::DECODE_ERROR);
    }

    #[test]
    fn decode_binary_frame_uncompressed_fallback() {
        let text = r#"{"op":1}"#;
        let decoded = decode_binary_frame(text.as_bytes(), None).unwrap();
        assert_eq!(decoded, text);
    }
}
