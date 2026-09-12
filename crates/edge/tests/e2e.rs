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
//! End-to-end tests exercising the fully wired gateway ([`edge::app::EdgeApp`]).
//!
//! These drive the real data path — an HTTP `POST` is validated, authorised,
//! de-duplicated, and published over GossipSub to an independent verifier node —
//! plus identity persistence across a restart. Only the HTTP → GossipSub path is
//! covered here; durable storage and IPFS are deferred subsystems.

use std::sync::Arc;
use std::time::Duration;

use edge::app::EdgeApp;
use edge::config::{AuthMode, Config, PubsubConfig};
use edge::p2p::{load_or_create_identity, GossipConfig, GossipNode, PeerRegistry};
use edge::pipeline::AcceptedMessage;
use edge::protocol::{decode_envelope, encode_envelope, sign_message, SensorIdentity, SignOptions};
use edge::shutdown::ShutdownController;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::timeout;

/// GossipSub topic shared by the gateway and the verifier node under test.
const TOPIC: &str = "sensors.social/e2e";
/// The plaintext payload signed into every test envelope.
const PAYLOAD: &[u8] = b"e2e-telemetry";

/// Build a gateway [`Config`] bound entirely to ephemeral loopback ports.
fn ephemeral_config() -> Config {
    let mut config = Config::default();
    config.http.enabled = true;
    config.http.listen = "127.0.0.1:0".parse().unwrap();
    config.auth.mode = AuthMode::None;
    config.pubsub.enabled = true;
    config.pubsub.listen = vec!["/ip4/127.0.0.1/tcp/0".to_string()];
    config.pubsub.topic = TOPIC.to_string();
    config.pubsub.min_connected_peers = 0;
    config.metrics.listen = "127.0.0.1:0".parse().unwrap();
    config
}

/// `POST` `body` to the gateway's ingress path, returning the HTTP status code.
///
/// Uses a raw connection to avoid pulling in an HTTP client dependency.
async fn post_ingress(addr: std::net::SocketAddr, body: &[u8]) -> u16 {
    let mut stream = TcpStream::connect(addr).await.expect("connect ingress");
    let request = format!(
        "POST /v1/telemetry HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write headers");
    stream.write_all(body).await.expect("write body");
    stream.flush().await.expect("flush");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");
    let text = String::from_utf8_lossy(&response);
    let status_line = text.lines().next().unwrap_or_default();
    status_line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

/// A freshly signed envelope carrying [`PAYLOAD`]; each call has a unique nonce
/// (so the pipeline's replay cache never suppresses it).
fn fresh_envelope(identity: &SensorIdentity) -> Vec<u8> {
    let envelope = sign_message(identity, PAYLOAD, SignOptions::default());
    encode_envelope(&envelope)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn http_ingress_is_published_over_gossip() {
    let app = EdgeApp::start(ephemeral_config())
        .await
        .expect("gateway starts");
    let http_addr = app.http_addr.expect("http bound");
    let gossip_listen = app.gossip_listen.clone().expect("gossip listening");

    // A standalone verifier node dialing the gateway as a reserved peer.
    let verifier_pubsub = PubsubConfig {
        enabled: true,
        listen: vec!["/ip4/127.0.0.1/tcp/0".to_string()],
        topic: TOPIC.to_string(),
        reserved_peers: vec![gossip_listen.to_string()],
        min_connected_peers: 0,
        identity_file: None,
    };
    let (inbound_tx, mut inbound_rx) = mpsc::channel(8);
    let verifier = GossipNode::new(
        load_or_create_identity(None).unwrap(),
        GossipConfig::from_pubsub(&verifier_pubsub).unwrap(),
        PeerRegistry::new(),
        None,
        Some(inbound_tx),
    )
    .expect("build verifier");
    let (_pub_tx, pub_rx) = mpsc::channel::<Arc<AcceptedMessage>>(1);
    let verifier_shutdown = ShutdownController::new();
    let verifier_task = tokio::spawn(verifier.run(pub_rx, verifier_shutdown.subscribe(), None));

    let identity = SensorIdentity::generate();

    // Re-post fresh envelopes until one propagates to the verifier: gossipsub
    // needs the connection and subscription mesh to form first.
    let received = timeout(Duration::from_secs(30), async {
        loop {
            let status = post_ingress(http_addr, &fresh_envelope(&identity)).await;
            assert_eq!(status, 202, "ingress should accept a valid envelope");
            if let Ok(Some(msg)) = timeout(Duration::from_millis(400), inbound_rx.recv()).await {
                break msg;
            }
        }
    })
    .await
    .expect("envelope delivered over gossip");

    let envelope = decode_envelope(&received.data).expect("published bytes are a valid envelope");
    assert_eq!(envelope.message, PAYLOAD);
    assert_eq!(
        envelope.sensor_id.as_slice(),
        identity.sensor_id().as_bytes().as_slice()
    );

    verifier_shutdown.trigger();
    let _ = verifier_task.await;
    app.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn identity_is_stable_across_restart() {
    let dir = std::env::temp_dir();
    let identity_file = dir.join(format!("edge-e2e-id-{}.hex", std::process::id()));
    let _ = std::fs::remove_file(&identity_file);

    let mut config = ephemeral_config();
    config.pubsub.identity_file = Some(identity_file.clone());

    let first = EdgeApp::start(config.clone()).await.expect("first start");
    let first_peer = first.gossip_peer_id.expect("peer id on first start");
    first.shutdown().await;

    let second = EdgeApp::start(config).await.expect("second start");
    let second_peer = second.gossip_peer_id.expect("peer id on second start");
    second.shutdown().await;

    assert_eq!(
        first_peer, second_peer,
        "a persisted identity must yield a stable peer id"
    );

    let _ = std::fs::remove_file(&identity_file);
}
