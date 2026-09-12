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
//! Connected-peer tracking and the `/debug/peers` diagnostics provider.
//!
//! [`PeerRegistry`] is a cheap, cloneable handle around a shared table of the
//! currently connected libp2p peers. The GossipSub node updates it as
//! connections open and close; the operations server reads a [`snapshot`] to
//! serve `/debug/peers`, and readiness gating compares
//! [`connected_count`](PeerRegistry::connected_count) against the configured
//! `min_connected_peers`.
//!
//! [`snapshot`]: PeerRegistry::snapshot

use libp2p::{Multiaddr, PeerId};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// A point-in-time view of one connected peer, serialized for `/debug/peers`.
#[derive(Clone, Debug, Serialize)]
pub struct PeerInfo {
    /// The peer's libp2p `PeerId`, as a base58 string.
    pub peer_id: String,
    /// The remote address of the (most recent) connection, if known.
    pub address: Option<String>,
    /// Unix-millisecond time the first connection to this peer was established.
    pub connected_since_ms: u64,
}

/// Internal per-peer bookkeeping.
#[derive(Debug)]
struct PeerEntry {
    /// Number of open connections to this peer (a peer may have several).
    connections: usize,
    /// Remote address of the most recently observed connection.
    address: Option<Multiaddr>,
    /// When the peer first connected (Unix milliseconds).
    since_ms: u64,
}

/// Shared, cloneable registry of connected peers.
#[derive(Clone, Default)]
pub struct PeerRegistry {
    /// Shared peer table guarded by a short-lived lock (never held across await).
    inner: Arc<RwLock<HashMap<PeerId, PeerEntry>>>,
}

impl PeerRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record an established connection to `peer` at `address`.
    ///
    /// Multiple connections to the same peer are reference-counted so the peer
    /// is only considered disconnected once its last connection closes.
    pub fn on_connected(&self, peer: PeerId, address: Option<Multiaddr>) {
        let mut table = self.inner.write().expect("peer registry poisoned");
        table
            .entry(peer)
            .and_modify(|entry| {
                entry.connections += 1;
                if address.is_some() {
                    entry.address = address.clone();
                }
            })
            .or_insert_with(|| PeerEntry {
                connections: 1,
                address,
                since_ms: now_ms(),
            });
    }

    /// Record a closed connection to `peer`, removing it once the count hits zero.
    pub fn on_disconnected(&self, peer: &PeerId) {
        let mut table = self.inner.write().expect("peer registry poisoned");
        if let Some(entry) = table.get_mut(peer) {
            entry.connections = entry.connections.saturating_sub(1);
            if entry.connections == 0 {
                table.remove(peer);
            }
        }
    }

    /// Number of distinct connected peers.
    pub fn connected_count(&self) -> usize {
        self.inner.read().expect("peer registry poisoned").len()
    }

    /// A sorted snapshot of connected peers for diagnostics output.
    pub fn snapshot(&self) -> Vec<PeerInfo> {
        let table = self.inner.read().expect("peer registry poisoned");
        let mut peers: Vec<PeerInfo> = table
            .iter()
            .map(|(peer, entry)| PeerInfo {
                peer_id: peer.to_base58(),
                address: entry.address.as_ref().map(ToString::to_string),
                connected_since_ms: entry.since_ms,
            })
            .collect();
        peers.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
        peers
    }
}

/// Current Unix time in milliseconds, saturating at the epoch.
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::identity::Keypair;

    fn peer_id() -> PeerId {
        Keypair::generate_ed25519().public().to_peer_id()
    }

    #[test]
    fn tracks_connect_and_disconnect() {
        let registry = PeerRegistry::new();
        let a = peer_id();
        let b = peer_id();
        assert_eq!(registry.connected_count(), 0);

        registry.on_connected(a, None);
        registry.on_connected(b, None);
        assert_eq!(registry.connected_count(), 2);

        registry.on_disconnected(&a);
        assert_eq!(registry.connected_count(), 1);
    }

    #[test]
    fn reference_counts_multiple_connections() {
        let registry = PeerRegistry::new();
        let a = peer_id();
        registry.on_connected(a, None);
        registry.on_connected(a, None);
        // Two connections: one close keeps the peer present.
        registry.on_disconnected(&a);
        assert_eq!(registry.connected_count(), 1);
        registry.on_disconnected(&a);
        assert_eq!(registry.connected_count(), 0);
    }

    #[test]
    fn snapshot_is_sorted_and_serializable() {
        let registry = PeerRegistry::new();
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/4001".parse().unwrap();
        registry.on_connected(peer_id(), Some(addr));

        let snapshot = registry.snapshot();
        assert_eq!(snapshot.len(), 1);
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(json.contains("peer_id"));
        assert!(json.contains("127.0.0.1"));
    }
}
