use crate::AppState;
use crate::constants::gateway::Opcode;

pub async fn handle(session_id: &str, state: &AppState) {
    if let Some(session) = state.hub.get(session_id) {
        session.touch_heartbeat();
        session.send(Opcode::HeartbeatAck, None);
    }
}
