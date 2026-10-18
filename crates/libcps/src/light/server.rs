///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2026 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
///////////////////////////////////////////////////////////////////////////////
//! Local WebSocket JSON-RPC server in front of the smoldot light client.
//!
//! Each accepted connection gets its own smoldot JSON-RPC frontend, so request
//! IDs and subscriptions of different clients never meet. Messages are passed
//! through unchanged; request bodies are never logged because they may carry
//! signed transactions.

use super::chain::{Chains, Frontend};
use futures_util::{SinkExt, StreamExt};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{watch, Semaphore},
    task::JoinSet,
};
use tokio_tungstenite::{
    tungstenite::{protocol::WebSocketConfig, Message},
    WebSocketStream,
};

/// Time a client has to complete the WebSocket handshake.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Pause after a failed `accept`, e.g. when out of file descriptors.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(100);

/// JSON-RPC error code of "server busy" style failures.
const BUSY_CODE: i64 = -32000;

/// Limits applied by the server.
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub(crate) max_connections: usize,
    pub(crate) max_message_size: usize,
}

/// Accept connections until `shutdown` fires; open connections are closed with the server.
pub(crate) async fn serve(
    listener: TcpListener,
    chains: Arc<Chains>,
    limits: Limits,
    mut shutdown: watch::Receiver<bool>,
) {
    let slots = Arc::new(Semaphore::new(limits.max_connections));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
            accepted = listener.accept() => match accepted {
                Ok((stream, peer)) => {
                    let Ok(slot) = slots.clone().try_acquire_owned() else {
                        tracing::warn!(%peer, "connection rejected: too many RPC clients");
                        continue;
                    };
                    let chains = chains.clone();
                    let shutdown = shutdown.clone();
                    connections.spawn(async move {
                        connection(stream, peer, chains, limits, shutdown).await;
                        drop(slot);
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to accept an RPC connection");
                    tokio::time::sleep(ACCEPT_BACKOFF).await;
                }
            },
        }
    }
    connections.shutdown().await;
}

/// Serve one client from handshake to disconnect.
async fn connection(
    stream: TcpStream,
    peer: SocketAddr,
    chains: Arc<Chains>,
    limits: Limits,
    shutdown: watch::Receiver<bool>,
) {
    let config = WebSocketConfig::default()
        .max_message_size(Some(limits.max_message_size))
        .max_frame_size(Some(limits.max_message_size));
    let handshake = tokio_tungstenite::accept_async_with_config(stream, Some(config));
    let ws = match tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake).await {
        Ok(Ok(ws)) => ws,
        Ok(Err(error)) => {
            tracing::debug!(%peer, %error, "WebSocket handshake failed");
            return;
        }
        Err(_) => {
            tracing::debug!(%peer, "WebSocket handshake timed out");
            return;
        }
    };

    // Opening a frontend parses the chain specification, keep it off the reactor.
    let frontend = match tokio::task::spawn_blocking(move || chains.open_frontend()).await {
        Ok(Ok(frontend)) => frontend,
        Ok(Err(error)) => {
            tracing::error!(%peer, %error, "cannot open a JSON-RPC session");
            return;
        }
        Err(error) => {
            tracing::error!(%peer, %error, "JSON-RPC session setup panicked");
            return;
        }
    };

    tracing::debug!(%peer, "RPC client connected");
    relay(ws, frontend, shutdown).await;
    tracing::debug!(%peer, "RPC client disconnected");
}

/// Pass messages between the client and its smoldot frontend.
async fn relay(
    ws: WebSocketStream<TcpStream>,
    mut frontend: Frontend,
    mut shutdown: watch::Receiver<bool>,
) {
    let (mut sink, mut stream) = ws.split();
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                let _ = sink.send(Message::Close(None)).await;
                break;
            }
            incoming = stream.next() => {
                let request = match incoming {
                    Some(Ok(Message::Text(text))) => text.as_str().to_owned(),
                    Some(Ok(Message::Binary(bytes))) => match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => text,
                        Err(_) => continue,
                    },
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                    Some(Ok(_)) => continue,
                };
                if let Err(rejected) = frontend.request(request) {
                    let smoldot_light::HandleRpcError::TooManyPendingRequests {
                        json_rpc_request,
                    } = rejected;
                    let reply = busy_reply(&json_rpc_request);
                    if sink.send(Message::text(reply)).await.is_err() {
                        break;
                    }
                }
            }
            response = frontend.next() => match response {
                Some(response) => {
                    if sink.send(Message::text(response)).await.is_err() {
                        break;
                    }
                }
                None => break,
            },
        }
    }
}

/// JSON-RPC error answering a request that was rejected because the queue is full.
fn busy_reply(request: &str) -> String {
    let id = serde_json::from_str::<serde_json::Value>(request)
        .ok()
        .and_then(|value| value.get("id").cloned())
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": BUSY_CODE, "message": "Too many pending requests" },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::busy_reply;

    #[test]
    fn busy_reply_keeps_request_id() {
        let reply = busy_reply(r#"{"jsonrpc":"2.0","id":"abc","method":"x","params":[]}"#);
        let value: serde_json::Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(value["id"], "abc");
        assert_eq!(value["error"]["code"], -32000);
    }

    #[test]
    fn busy_reply_of_garbage_has_null_id() {
        let value: serde_json::Value = serde_json::from_str(&busy_reply("not json")).unwrap();
        assert!(value["id"].is_null());
    }
}
