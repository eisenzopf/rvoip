//! WebSocket real-time event hub.
//!
//! Manages WebSocket connections and broadcasts events from the
//! call-engine event system to connected frontend clients.

pub mod hub;

pub use hub::WsHub;

use axum::{
    Router,
    routing::get,
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    response::Response,
};
use futures::{SinkExt, StreamExt};
use tracing::{info, warn};

use crate::server::AppState;

/// WebSocket upgrade handler.
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> Response {
    ws.on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Subscribe to the broadcast channel
    let mut rx = state.ws_hub.subscribe();
    let client_id = uuid::Uuid::new_v4().to_string();

    info!(client_id = %client_id, "WebSocket client connected");

    // Spawn a task to forward broadcast messages to this client
    let send_task = tokio::spawn(async move {
        while let Ok(msg) = rx.recv().await {
            if sender.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    });

    // Read incoming messages (for future: client commands like subscribe/filter)
    while let Some(Ok(msg)) = receiver.next().await {
        match msg {
            Message::Text(text) => {
                // Could handle client filter/subscribe commands here
                let _ = text;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    warn!(client_id = %client_id, "WebSocket client disconnected");
    send_task.abort();
}

pub fn router() -> Router<AppState> {
    Router::new().route("/events", get(ws_handler))
}
