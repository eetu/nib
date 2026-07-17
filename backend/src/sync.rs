//! Live op-sync (Phase C2) — a WebSocket per project. Clients (the browser; other tools) connect to
//! `GET /ws/projects/{id}?token=…`, authenticate, and attach to the project's session. Ops from any
//! client (WS *or* MCP) funnel through [`session::apply_ops`], which broadcasts them to every other
//! subscriber — so the LLM's edits appear on the canvas live, and vice-versa. Messages are
//! `{ clientId, ops }`; a client ignores the echo of its own `clientId`.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;
use tokio::sync::broadcast::error::RecvError;

use crate::AppState;
use crate::db;
use crate::session::{self, SyncMsg};

#[derive(Deserialize)]
pub struct WsAuth {
    /// Bearer token as a query param — browsers can't set headers on a WebSocket handshake.
    pub token: String,
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Path(id): Path<i64>,
    Query(q): Query<WsAuth>,
    State(st): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, id, q.token, st))
}

async fn handle_socket(mut socket: WebSocket, project_id: i64, token: String, st: AppState) {
    // Authenticate + verify the caller owns the project (open loads/attaches its session).
    let Some(user) = db::user_by_token(&st.pool, &token).await.ok().flatten() else {
        let _ = socket.send(Message::Close(None)).await;
        return;
    };
    let sess = match session::open(&st.pool, &st.sessions, user.id, project_id).await {
        Ok(s) => s,
        Err(_) => {
            let _ = socket.send(Message::Close(None)).await;
            return;
        }
    };
    let mut rx = sess.lock().unwrap().tx.subscribe();

    loop {
        tokio::select! {
            // A broadcast op batch (from another client, or MCP) → forward to this socket.
            bc = rx.recv() => match bc {
                Ok(msg) => {
                    let Ok(txt) = serde_json::to_string(&msg) else { continue };
                    if socket.send(Message::Text(txt.into())).await.is_err() {
                        break;
                    }
                }
                Err(RecvError::Lagged(_)) => continue, // slow client dropped some — keep going
                Err(RecvError::Closed) => break,
            },
            // An op batch from this client → apply through the session funnel (persists + broadcasts).
            ws = socket.recv() => match ws {
                Some(Ok(Message::Text(t))) => {
                    if let Ok(msg) = serde_json::from_str::<SyncMsg>(t.as_str()) {
                        let _ = session::apply_ops(&sess, &st.pool, msg.ops, &msg.client_id);
                    }
                }
                Some(Ok(Message::Close(_))) | None => break,
                Some(Ok(_)) => {}          // ignore ping/pong/binary
                Some(Err(_)) => break,
            },
        }
    }
}
