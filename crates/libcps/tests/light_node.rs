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
//! Tests of the embedded light node (`libcps::light`).
//!
//! The tests start a real node on an ephemeral loopback port and talk plain
//! WebSocket JSON-RPC to it. They need no synchronized chain: smoldot answers
//! chain-independent requests right away. The test that waits for finality
//! needs network access and is `#[ignore]`d:
//!
//! ```text
//! cargo test -p libcps --test light_node --no-default-features --features light-client \
//!     -- --ignored
//! ```

use futures_util::{SinkExt, StreamExt};
use libcps::light::{LightError, LightNode, LightNodeConfig, LightStatus};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{tungstenite::Message, WebSocketStream};

type Ws = WebSocketStream<TcpStream>;

const TIMEOUT: Duration = Duration::from_secs(60);

async fn start() -> LightNode {
    LightNode::start(LightNodeConfig::default().listen("127.0.0.1:0"))
        .await
        .expect("light node starts")
}

async fn connect(node: &LightNode) -> Ws {
    let stream = TcpStream::connect(node.rpc_addr()).await.unwrap();
    let (ws, _) = tokio_tungstenite::client_async(node.rpc_url(), stream)
        .await
        .expect("WebSocket handshake");
    ws
}

/// Send a request and return the response carrying the same `id`.
async fn call(ws: &mut Ws, id: u64, method: &str, params: Value) -> Value {
    let request = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
    ws.send(Message::text(request.to_string())).await.unwrap();
    tokio::time::timeout(TIMEOUT, async {
        loop {
            let Message::Text(text) = ws.next().await.expect("connection open").unwrap() else {
                continue;
            };
            let value: Value = serde_json::from_str(text.as_str()).unwrap();
            if value["id"] == json!(id) {
                return value;
            }
        }
    })
    .await
    .expect("response in time")
}

#[tokio::test]
async fn starts_on_an_ephemeral_loopback_port() {
    let node = start().await;
    let addr = node.rpc_addr();
    assert!(addr.ip().is_loopback());
    assert_ne!(addr.port(), 0);
    assert_eq!(node.rpc_url(), format!("ws://{addr}"));
    assert_ne!(node.current_status(), LightStatus::Starting);
    assert!(!node.current_status().is_terminal());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn answers_basic_rpc_requests() {
    let node = start().await;
    let mut ws = connect(&node).await;

    let chain = call(&mut ws, 1, "system_chain", json!([])).await;
    assert!(chain["result"].is_string(), "unexpected reply: {chain}");

    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn shutdown_closes_clients_and_releases_the_port() {
    let node = start().await;
    let addr = node.rpc_addr();
    let mut ws = connect(&node).await;
    call(&mut ws, 1, "system_chain", json!([])).await;

    node.shutdown().await.unwrap();

    // The client observes the end of the connection.
    let end = tokio::time::timeout(TIMEOUT, async {
        while let Some(Ok(_)) = ws.next().await {}
    })
    .await;
    assert!(end.is_ok(), "connection was not closed");
    TcpListener::bind(addr).await.expect("port is free again");
}

#[tokio::test]
async fn status_is_stopped_after_shutdown() {
    let node = start().await;
    let mut status = node.status();
    node.shutdown().await.unwrap();
    status.wait_for(LightStatus::is_terminal).await.unwrap();
    assert_eq!(*status.borrow(), LightStatus::Stopped);
}

#[tokio::test]
async fn clients_do_not_collide_on_request_ids() {
    let node = start().await;
    let mut first = connect(&node).await;
    let mut second = connect(&node).await;

    // Same request ID on both connections, different requests.
    let (chain, name) = tokio::join!(
        call(&mut first, 1, "system_chain", json!([])),
        call(&mut second, 1, "system_name", json!([])),
    );
    assert_eq!(chain["id"], 1);
    assert_eq!(name["id"], 1);
    assert_ne!(chain["result"], name["result"]);

    // Each connection keeps its own answer for the same method too.
    let (a, b) = tokio::join!(
        call(&mut first, 7, "system_chain", json!([])),
        call(&mut second, 7, "system_chain", json!([])),
    );
    assert_eq!(a["result"], b["result"]);

    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn subscriptions_do_not_leak_between_clients() {
    let node = start().await;
    let mut owner = connect(&node).await;
    let mut stranger = connect(&node).await;

    let subscribed = call(&mut owner, 1, "chain_subscribeNewHeads", json!([])).await;
    let subscription = subscribed["result"].clone();
    assert!(subscription.is_string(), "unexpected reply: {subscribed}");

    // Another client cannot cancel the subscription of the owner.
    let foreign = call(
        &mut stranger,
        1,
        "chain_unsubscribeNewHeads",
        json!([subscription]),
    )
    .await;
    assert_ne!(foreign["result"], json!(true), "leaked: {foreign}");

    // The owner still can.
    let own = call(
        &mut owner,
        2,
        "chain_unsubscribeNewHeads",
        json!([subscription]),
    )
    .await;
    assert_eq!(own["result"], json!(true), "unexpected reply: {own}");

    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn closed_clients_do_not_stop_the_node() {
    let node = start().await;
    let mut gone = connect(&node).await;
    call(&mut gone, 1, "system_chain", json!([])).await;
    drop(gone);

    let mut other = connect(&node).await;
    let reply = call(&mut other, 1, "system_chain", json!([])).await;
    assert!(reply["result"].is_string());

    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn occupied_listen_address_is_a_bind_error() {
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = occupied.local_addr().unwrap();

    let error = LightNode::start(LightNodeConfig::default().listen(addr))
        .await
        .err()
        .expect("address is in use");
    assert!(matches!(error, LightError::Bind { .. }), "got {error:?}");
    assert!(error.to_string().contains(&addr.to_string()));
}

#[tokio::test]
async fn non_loopback_listen_needs_explicit_opt_in() {
    let error = LightNode::start(LightNodeConfig::default().listen("0.0.0.0:0"))
        .await
        .err()
        .expect("non-loopback is rejected by default");
    assert!(matches!(error, LightError::Config(_)), "got {error:?}");

    let node = LightNode::start(
        LightNodeConfig::default()
            .listen("0.0.0.0:0")
            .allow_non_loopback(true),
    )
    .await
    .expect("explicitly allowed");
    // The URL is always usable by a local client.
    assert!(node.rpc_url().starts_with("ws://127.0.0.1:"));
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn default_config_listens_on_loopback() {
    let node = LightNode::start(LightNodeConfig::default()).await.unwrap();
    assert!(node.rpc_addr().ip().is_loopback());
    node.shutdown().await.unwrap();
}

#[tokio::test]
async fn invalid_chain_spec_is_reported() {
    let error = LightNode::start(LightNodeConfig::default().chain_specs("{}", "{}"))
        .await
        .err()
        .expect("specification is invalid");
    assert!(
        matches!(error, LightError::InvalidChainSpec(_)),
        "got {error:?}"
    );
}

#[tokio::test]
#[ignore = "requires network access and takes a while"]
async fn syncs_and_serves_cps() {
    let node = start().await;
    tokio::time::timeout(Duration::from_secs(900), node.wait_ready())
        .await
        .expect("synchronized in time")
        .unwrap();

    let cps = libcps::Cps::connect(node.rpc_url()).await.unwrap();
    let at = cps.finalized().await.unwrap();
    assert!(!at.block_hash().as_ref().is_empty());

    node.shutdown().await.unwrap();
}
