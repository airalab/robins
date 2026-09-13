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
//! Embedded libp2p GossipSub publisher.
//!
//! The gateway runs a small libp2p node whose sole application job is to publish
//! accepted envelopes to a GossipSub topic. It:
//!
//! - derives a **stable** peer identity from an on-disk Ed25519 secret (created
//!   on first run), so the node keeps the same `PeerId` across restarts;
//! - dials the configured **reserved peers** and periodically re-dials any that
//!   are disconnected;
//! - tracks connected peers in a [`PeerRegistry`] (for `/debug/peers`), exports
//!   the [`CONNECTED_PEERS`] gauge, and drives readiness from the configured
//!   `min_connected_peers` threshold;
//! - consumes [`AcceptedMessage`]s from the pipeline and publishes their exact
//!   envelope bytes, incrementing [`PUBLISH_TOTAL`] on success.
//!
//! Identify and ping run alongside GossipSub for peer metadata and keep-alive.
//!
//! The node listens and dials over TCP as well as plain (`/ws`) and secure
//! (`/wss`) WebSocket transports, all upgraded with noise + yamux, so peers
//! reachable only via TLS-terminated WebSocket endpoints can be used.

pub mod peers;

pub use peers::{PeerInfo, PeerRegistry};

use crate::config::PubsubConfig;
use crate::observability::metrics::{CONNECTED_PEERS, PUBLISH_TOTAL};
use crate::observability::Health;
use crate::pipeline::AcceptedMessage;
use crate::shutdown::ShutdownSignal;
use futures::StreamExt;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::{gossipsub, identify, noise, ping, tcp, yamux, Multiaddr, PeerId, Swarm};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

/// libp2p protocol name advertised over the identify protocol.
const IDENTIFY_PROTOCOL: &str = "/edge/id/1.0.0";
/// How often disconnected reserved peers are re-dialed.
const RECONNECT_INTERVAL: Duration = Duration::from_secs(15);
/// How long an idle connection is kept before libp2p closes it.
const IDLE_CONNECTION_TIMEOUT: Duration = Duration::from_secs(60);
/// Length in bytes of the persisted Ed25519 node secret.
const SECRET_LEN: usize = 32;

/// The composed network behaviour: GossipSub plus identify and ping.
#[derive(NetworkBehaviour)]
struct EdgeBehaviour {
    /// Publish/subscribe message routing.
    gossipsub: gossipsub::Behaviour,
    /// Peer metadata exchange.
    identify: identify::Behaviour,
    /// Liveness keep-alive.
    ping: ping::Behaviour,
}

/// Parsed, validated GossipSub node configuration.
#[derive(Clone, Debug)]
pub struct GossipConfig {
    /// Topic accepted envelopes are published to.
    pub topic: String,
    /// Addresses to listen on.
    pub listen: Vec<Multiaddr>,
    /// Reserved peers to dial and keep connected.
    pub reserved_peers: Vec<Multiaddr>,
    /// Connected-peer threshold gating readiness.
    pub min_connected_peers: usize,
}

impl GossipConfig {
    /// Parse the `[pubsub]` configuration section, validating multiaddresses.
    pub fn from_pubsub(config: &PubsubConfig) -> std::io::Result<Self> {
        let listen = parse_multiaddrs(&config.listen)?;
        let reserved_peers = parse_multiaddrs(&config.reserved_peers)?;
        Ok(Self {
            topic: config.topic.clone(),
            listen,
            reserved_peers,
            min_connected_peers: config.min_connected_peers,
        })
    }
}

/// Parse a list of string multiaddresses, reporting the offending entry.
fn parse_multiaddrs(values: &[String]) -> std::io::Result<Vec<Multiaddr>> {
    values
        .iter()
        .map(|value| {
            value.parse::<Multiaddr>().map_err(|err| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("invalid multiaddr '{value}': {err}"),
                )
            })
        })
        .collect()
}

/// A message received from the GossipSub network (diagnostic / test hook).
#[derive(Clone, Debug)]
pub struct ReceivedMessage {
    /// The peer that propagated the message, if reported.
    pub source: Option<PeerId>,
    /// The raw message payload (an envelope's wire bytes).
    pub data: Vec<u8>,
}

/// Load a stable Ed25519 identity, generating and persisting one if absent.
///
/// When `identity_file` is `None` an ephemeral identity is generated (useful for
/// tests and throwaway nodes). Otherwise the 32-byte secret is read as
/// `0x`-prefixed hex from the file, or created and written on first run.
pub fn load_or_create_identity(
    identity_file: Option<&std::path::Path>,
) -> std::io::Result<libp2p::identity::Keypair> {
    use libp2p::identity::Keypair;
    use rand::RngCore;

    let Some(path) = identity_file else {
        return Ok(Keypair::generate_ed25519());
    };

    if path.exists() {
        let encoded = std::fs::read_to_string(path)?;
        let stripped = crate::protocol::strip_0x(encoded.trim()).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid identity hex in {}: {err}", path.display()),
            )
        })?;
        let mut bytes = hex::decode(stripped).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid identity hex in {}: {err}", path.display()),
            )
        })?;
        if bytes.len() != SECRET_LEN {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "identity {} must be {SECRET_LEN} bytes, got {}",
                    path.display(),
                    bytes.len()
                ),
            ));
        }
        Keypair::ed25519_from_bytes(&mut bytes).map_err(|err| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid ed25519 identity in {}: {err}", path.display()),
            )
        })
    } else {
        let mut seed = [0u8; SECRET_LEN];
        rand::rngs::OsRng.fill_bytes(&mut seed);
        std::fs::write(path, format!("0x{}", hex::encode(seed)))?;
        Keypair::ed25519_from_bytes(seed).map_err(|err| {
            std::io::Error::other(format!("failed to build ed25519 identity: {err}"))
        })
    }
}

/// The running GossipSub node.
pub struct GossipNode {
    /// The libp2p swarm.
    swarm: Swarm<EdgeBehaviour>,
    /// Topic accepted envelopes are published to.
    topic: gossipsub::IdentTopic,
    /// Reserved peers to keep connected.
    reserved_peers: Vec<Multiaddr>,
    /// Readiness threshold on connected peers.
    min_connected_peers: usize,
    /// Shared connected-peer registry (diagnostics + readiness).
    peers: PeerRegistry,
    /// Optional readiness handle updated as peers connect/disconnect.
    health: Option<Health>,
    /// Optional forwarding of received messages (diagnostics / tests).
    inbound: Option<mpsc::Sender<ReceivedMessage>>,
}

impl GossipNode {
    /// Build a node from a keypair and parsed configuration.
    ///
    /// The node subscribes to the topic and begins listening/dialing when
    /// [`run`](Self::run) is awaited.
    pub async fn new(
        keypair: libp2p::identity::Keypair,
        config: GossipConfig,
        peers: PeerRegistry,
        health: Option<Health>,
        inbound: Option<mpsc::Sender<ReceivedMessage>>,
    ) -> std::io::Result<Self> {
        let mut swarm = build_swarm(keypair).await?;

        let topic = gossipsub::IdentTopic::new(config.topic.clone());
        swarm
            .behaviour_mut()
            .gossipsub
            .subscribe(&topic)
            .map_err(|err| {
                std::io::Error::other(format!("failed to subscribe to '{}': {err}", config.topic))
            })?;

        for addr in &config.listen {
            swarm.listen_on(addr.clone()).map_err(|err| {
                std::io::Error::other(format!("failed to listen on {addr}: {err}"))
            })?;
        }

        Ok(Self {
            swarm,
            topic,
            reserved_peers: config.reserved_peers,
            min_connected_peers: config.min_connected_peers,
            peers,
            health,
            inbound,
        })
    }

    /// The node's stable local peer id.
    pub fn local_peer_id(&self) -> PeerId {
        *self.swarm.local_peer_id()
    }

    /// Run the node until `shutdown` fires, publishing messages from `publish_rx`.
    ///
    /// The node dials reserved peers on startup and re-dials disconnected ones on
    /// a fixed interval. An optional `listen_addrs` sender receives each bound
    /// listen address (used by tests to discover ephemeral ports).
    pub async fn run(
        mut self,
        mut publish_rx: mpsc::Receiver<Arc<AcceptedMessage>>,
        mut shutdown: ShutdownSignal,
        listen_addrs: Option<mpsc::Sender<Multiaddr>>,
    ) {
        self.dial_reserved();
        self.refresh_readiness();

        let mut reconnect = tokio::time::interval(RECONNECT_INTERVAL);
        reconnect.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        tracing::info!(peer_id = %self.local_peer_id(), "gossip node started");

        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    tracing::info!("gossip node shutting down");
                    break;
                }
                _ = reconnect.tick() => self.dial_reserved(),
                message = publish_rx.recv() => match message {
                    Some(message) => self.publish(&message),
                    None => {
                        tracing::info!("publish channel closed; gossip node stopping");
                        break;
                    }
                },
                event = self.swarm.select_next_some() => {
                    self.on_swarm_event(event, listen_addrs.as_ref());
                }
            }
        }
    }

    /// Publish an accepted message's exact envelope bytes to the topic.
    fn publish(&mut self, message: &AcceptedMessage) {
        let data = message.raw_envelope.to_vec();
        match self
            .swarm
            .behaviour_mut()
            .gossipsub
            .publish(self.topic.hash(), data)
        {
            Ok(_) => {
                metrics::counter!(PUBLISH_TOTAL).increment(1);
                tracing::info!(envelope_id = %message.envelope_id, "published to gossipsub");
            }
            Err(gossipsub::PublishError::InsufficientPeers) => {
                tracing::warn!(
                    envelope_id = %message.envelope_id,
                    "no subscribed peers; message not published"
                );
            }
            Err(gossipsub::PublishError::Duplicate) => {
                tracing::trace!(envelope_id = %message.envelope_id, "duplicate publish ignored");
            }
            Err(err) => {
                tracing::warn!(envelope_id = %message.envelope_id, %err, "publish failed");
            }
        }
    }

    /// Dial every reserved peer, ignoring already-connected/in-progress dials.
    fn dial_reserved(&mut self) {
        for addr in self.reserved_peers.clone() {
            match self.swarm.dial(addr.clone()) {
                Ok(()) => tracing::debug!(%addr, "dialing reserved peer"),
                Err(err) => tracing::trace!(%addr, %err, "reserved dial skipped"),
            }
        }
    }

    /// Handle a single swarm event.
    fn on_swarm_event(
        &mut self,
        event: SwarmEvent<EdgeBehaviourEvent>,
        listen_addrs: Option<&mpsc::Sender<Multiaddr>>,
    ) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                tracing::info!(%address, "gossip node listening");
                if let Some(tx) = listen_addrs {
                    let _ = tx.try_send(address);
                }
            }
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                self.peers
                    .on_connected(peer_id, Some(endpoint.get_remote_address().clone()));
                self.refresh_readiness();
                tracing::debug!(%peer_id, count = self.peers.connected_count(), "peer connected");
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                self.peers.on_disconnected(&peer_id);
                self.refresh_readiness();
                tracing::debug!(%peer_id, count = self.peers.connected_count(), "peer disconnected");
            }
            SwarmEvent::Behaviour(EdgeBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                message,
                ..
            })) => {
                if let Some(tx) = &self.inbound {
                    let _ = tx.try_send(ReceivedMessage {
                        source: message.source,
                        data: message.data,
                    });
                }
            }
            _ => {}
        }
    }

    /// Recompute the connected-peer gauge and readiness flag.
    fn refresh_readiness(&self) {
        let count = self.peers.connected_count();
        metrics::gauge!(CONNECTED_PEERS).set(count as f64);
        if let Some(health) = &self.health {
            health.set_ready(count >= self.min_connected_peers);
        }
    }
}

/// Construct the swarm with TCP + WebSocket(Secure) transports (noise + yamux)
/// and the edge behaviour.
///
/// Both plain (`/ws`) and secure (`/wss`) WebSocket multiaddresses are supported
/// in addition to raw `/tcp`, so the gateway can dial peers reached over
/// TLS-terminated WebSocket endpoints (a common relay/ingress topology).
async fn build_swarm(keypair: libp2p::identity::Keypair) -> std::io::Result<Swarm<EdgeBehaviour>> {
    fn to_io(err: impl std::fmt::Display) -> std::io::Error {
        std::io::Error::other(err.to_string())
    }

    let swarm = libp2p::SwarmBuilder::with_existing_identity(keypair)
        .with_tokio()
        .with_tcp(
            tcp::Config::default().nodelay(true),
            noise::Config::new,
            yamux::Config::default,
        )
        .map_err(to_io)?
        .with_websocket(noise::Config::new, yamux::Config::default)
        .await
        .map_err(to_io)?
        .with_behaviour(build_behaviour)
        .map_err(to_io)?
        .with_swarm_config(|config| config.with_idle_connection_timeout(IDLE_CONNECTION_TIMEOUT))
        .build();

    Ok(swarm)
}

/// Build the composed behaviour from the node's keypair.
fn build_behaviour(
    keypair: &libp2p::identity::Keypair,
) -> Result<EdgeBehaviour, Box<dyn std::error::Error + Send + Sync>> {
    // Deduplicate by content: the message id is the SHA-256 of the payload, which
    // matches the gateway's envelope-id convention so identical envelopes are
    // suppressed network-wide regardless of who forwards them.
    let message_id_fn = |message: &gossipsub::Message| {
        let digest = Sha256::digest(&message.data);
        gossipsub::MessageId::from(digest.to_vec())
    };

    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Strict)
        .message_id_fn(message_id_fn)
        .build()?;

    let gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(keypair.clone()),
        gossipsub_config,
    )?;

    let identify = identify::Behaviour::new(identify::Config::new(
        IDENTIFY_PROTOCOL.to_string(),
        keypair.public(),
    ));

    Ok(EdgeBehaviour {
        gossipsub,
        identify,
        ping: ping::Behaviour::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingress::TransportMetadata;
    use crate::protocol::{
        encode_envelope, envelope_id, sign_message, SensorIdentity, SignOptions,
    };
    use bytes::Bytes;
    use std::time::SystemTime;
    use tokio::time::{timeout, Duration};

    fn accepted_message() -> Arc<AcceptedMessage> {
        let identity = SensorIdentity::from_secret_bytes(&[42u8; 32]);
        let envelope = sign_message(&identity, b"telemetry", SignOptions::default());
        let raw = Bytes::from(encode_envelope(&envelope));
        Arc::new(AcceptedMessage {
            sensor_id: identity.sensor_id(),
            envelope_id: envelope_id(&raw),
            envelope,
            raw_envelope: raw,
            transport: TransportMetadata::http(None),
            received_at: SystemTime::now(),
        })
    }

    fn test_config(listen: Vec<Multiaddr>, reserved: Vec<Multiaddr>) -> GossipConfig {
        GossipConfig {
            topic: "edge.test/v1".to_string(),
            listen,
            reserved_peers: reserved,
            min_connected_peers: 1,
        }
    }

    #[test]
    fn ephemeral_identity_is_generated() {
        let key = load_or_create_identity(None).unwrap();
        assert!(!key.public().to_peer_id().to_base58().is_empty());
    }

    #[test]
    fn identity_persists_across_loads() {
        let dir = std::env::temp_dir().join(format!("edge-id-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("identity.hex");
        let _ = std::fs::remove_file(&path);

        let first = load_or_create_identity(Some(&path)).unwrap();
        let second = load_or_create_identity(Some(&path)).unwrap();
        assert_eq!(
            first.public().to_peer_id(),
            second.public().to_peer_id(),
            "identity must be stable across restarts"
        );
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn rejects_invalid_multiaddr() {
        let config = PubsubConfig {
            listen: vec!["not-a-multiaddr".to_string()],
            ..PubsubConfig::default()
        };
        assert!(GossipConfig::from_pubsub(&config).is_err());
    }

    // Two nodes on loopback: one dials the other, both subscribe, and a message
    // published by the first is delivered to the second over GossipSub.
    #[tokio::test]
    async fn two_nodes_exchange_a_published_message() {
        let listen: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();

        // Node A: the publisher. Discover its bound address via `listen_addrs`.
        let (addr_tx, mut addr_rx) = mpsc::channel(4);
        let node_a = GossipNode::new(
            load_or_create_identity(None).unwrap(),
            test_config(vec![listen.clone()], vec![]),
            PeerRegistry::new(),
            None,
            None,
        )
        .await
        .unwrap();
        let (publish_tx, publish_rx) = mpsc::channel(8);
        let shutdown_a = crate::shutdown::ShutdownController::new();
        let sig_a = shutdown_a.subscribe();
        let handle_a = tokio::spawn(node_a.run(publish_rx, sig_a, Some(addr_tx)));

        let bound = timeout(Duration::from_secs(5), addr_rx.recv())
            .await
            .expect("node A listen address")
            .expect("listen address present");

        // Node B: the subscriber, dialing node A as a reserved peer.
        let (inbound_tx, mut inbound_rx) = mpsc::channel(8);
        let node_b = GossipNode::new(
            load_or_create_identity(None).unwrap(),
            test_config(vec![listen], vec![bound]),
            PeerRegistry::new(),
            None,
            Some(inbound_tx),
        )
        .await
        .unwrap();
        let (_publish_tx_b, publish_rx_b) = mpsc::channel::<Arc<AcceptedMessage>>(8);
        let shutdown_b = crate::shutdown::ShutdownController::new();
        let sig_b = shutdown_b.subscribe();
        let handle_b = tokio::spawn(node_b.run(publish_rx_b, sig_b, None));

        // Re-publish until B receives, allowing time for connect + subscription.
        let message = accepted_message();
        let received = timeout(Duration::from_secs(20), async {
            loop {
                let _ = publish_tx.send(Arc::clone(&message)).await;
                if let Ok(Some(received)) =
                    timeout(Duration::from_millis(300), inbound_rx.recv()).await
                {
                    break received;
                }
            }
        })
        .await
        .expect("message delivered to node B");

        assert_eq!(received.data, message.raw_envelope.to_vec());

        shutdown_a.trigger();
        shutdown_b.trigger();
        let _ = handle_a.await;
        let _ = handle_b.await;
    }

    // Same delivery guarantee as above, but both nodes speak the WebSocket
    // transport (`/ws`), exercising the `with_websocket` builder branch that also
    // provides `/wss` dialing.
    #[tokio::test]
    async fn two_nodes_exchange_over_websocket() {
        let listen: Multiaddr = "/ip4/127.0.0.1/tcp/0/ws".parse().unwrap();

        let (addr_tx, mut addr_rx) = mpsc::channel(4);
        let node_a = GossipNode::new(
            load_or_create_identity(None).unwrap(),
            test_config(vec![listen.clone()], vec![]),
            PeerRegistry::new(),
            None,
            None,
        )
        .await
        .unwrap();
        let (publish_tx, publish_rx) = mpsc::channel(8);
        let shutdown_a = crate::shutdown::ShutdownController::new();
        let handle_a = tokio::spawn(node_a.run(publish_rx, shutdown_a.subscribe(), Some(addr_tx)));

        // The reported listen address must carry the `/ws` protocol.
        let bound = timeout(Duration::from_secs(5), addr_rx.recv())
            .await
            .expect("node A listen address")
            .expect("listen address present");
        assert!(
            bound
                .iter()
                .any(|p| matches!(p, libp2p::multiaddr::Protocol::Ws(_))),
            "expected a websocket listen address, got {bound}"
        );

        let (inbound_tx, mut inbound_rx) = mpsc::channel(8);
        let node_b = GossipNode::new(
            load_or_create_identity(None).unwrap(),
            test_config(vec![listen], vec![bound]),
            PeerRegistry::new(),
            None,
            Some(inbound_tx),
        )
        .await
        .unwrap();
        let (_publish_tx_b, publish_rx_b) = mpsc::channel::<Arc<AcceptedMessage>>(8);
        let shutdown_b = crate::shutdown::ShutdownController::new();
        let handle_b = tokio::spawn(node_b.run(publish_rx_b, shutdown_b.subscribe(), None));

        let message = accepted_message();
        let received = timeout(Duration::from_secs(20), async {
            loop {
                let _ = publish_tx.send(Arc::clone(&message)).await;
                if let Ok(Some(received)) =
                    timeout(Duration::from_millis(300), inbound_rx.recv()).await
                {
                    break received;
                }
            }
        })
        .await
        .expect("message delivered over websocket");

        assert_eq!(received.data, message.raw_envelope.to_vec());

        shutdown_a.trigger();
        shutdown_b.trigger();
        let _ = handle_a.await;
        let _ = handle_b.await;
    }
}
