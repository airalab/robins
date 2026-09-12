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
//! The canonical acceptance pipeline: verify → authorize → deduplicate → fan-out.
//!
//! Every ingress transport funnels [`IngressMessage`]s into a single bounded
//! channel; this module drains that channel and applies the one canonical
//! sequence of checks, so transports can never diverge in policy:
//!
//! 1. **Verify** the Ed25519 signature ([`verify_envelope`]); on failure the
//!    message is dropped and [`VALIDATION_REJECTED_TOTAL`] is incremented.
//! 2. **Authorize** the verified `sensor_id` against the configured
//!    [`AuthPolicy`]; rejects increment [`AUTH_REJECTED_TOTAL`].
//! 3. **Deduplicate** by [`EnvelopeId`] via a bounded TTL [`DedupCache`];
//!    duplicates increment [`DEDUP_DROPPED_TOTAL`].
//! 4. **Fan out** the resulting [`AcceptedMessage`] to every configured
//!    [`MessageSink`] (live GossipSub today; a durable IPFS spool later).
//!
//! Fan-out awaits each sink, so a slow or bounded sink backpressures the whole
//! pipeline, which in turn backpressures ingress transports (e.g. HTTP `429`).
//! Sinks are kept decoupled: one sink's failure never aborts delivery to the
//! others, preserving independent failure domains between the live and durable
//! paths.

pub mod dedup;

pub use dedup::DedupCache;

use crate::auth::AuthPolicy;
use crate::ingress::{IngressMessage, IngressReceiver, TransportMetadata};
use crate::observability::metrics::{
    AUTH_REJECTED_TOTAL, DEDUP_DROPPED_TOTAL, VALIDATION_REJECTED_TOTAL,
};
use crate::protocol::{envelope_id, verify_envelope, EnvelopeId, SensorId, SignedEnvelope};
use crate::shutdown::ShutdownSignal;
use async_trait::async_trait;
use bytes::Bytes;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::mpsc;

/// A fully validated, authorized and deduplicated message ready for delivery.
///
/// This is the frozen hand-off type between the pipeline and its sinks. It
/// carries the verified [`SensorId`], the decoded envelope, the exact wire
/// bytes (for canonical re-publishing) and the precomputed [`EnvelopeId`].
#[derive(Clone, Debug)]
pub struct AcceptedMessage {
    /// The cryptographically verified sensor identity.
    pub sensor_id: SensorId,
    /// The verified envelope.
    pub envelope: SignedEnvelope,
    /// The exact bytes the envelope was decoded from.
    pub raw_envelope: Bytes,
    /// Content id (SHA-256 of `raw_envelope`); the dedup/publish key.
    pub envelope_id: EnvelopeId,
    /// How the message originally arrived (diagnostic only).
    pub transport: TransportMetadata,
    /// When the message was accepted at the ingress boundary.
    pub received_at: SystemTime,
}

/// Error returned by a [`MessageSink`] when it cannot accept a message.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// The sink's downstream consumer has gone away.
    #[error("sink '{0}' is closed")]
    Closed(&'static str),
    /// A sink-specific delivery failure.
    #[error("sink '{sink}' delivery failed: {source}")]
    Delivery {
        /// The sink that failed.
        sink: &'static str,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// A downstream consumer of [`AcceptedMessage`]s.
///
/// Sinks are the pipeline's fan-out targets: the live GossipSub publisher and,
/// in later phases, the durable IPFS spool. `deliver` takes an [`Arc`] so a
/// single accepted message can be shared across sinks without cloning payloads.
#[async_trait]
pub trait MessageSink: Send + Sync {
    /// A stable, human-readable sink name for logs and diagnostics.
    fn name(&self) -> &'static str;

    /// Deliver `message` downstream, awaiting to apply backpressure.
    async fn deliver(&self, message: Arc<AcceptedMessage>) -> Result<(), SinkError>;
}

/// A [`MessageSink`] that forwards accepted messages over a bounded channel.
///
/// This is the seam other subsystems plug into: the GossipSub publisher (and,
/// later, the durable spool) own the receiving half and consume at their own
/// pace. A bounded channel provides backpressure; a closed channel surfaces as
/// [`SinkError::Closed`].
pub struct ChannelSink {
    /// Stable sink name.
    name: &'static str,
    /// Sending half of the downstream channel.
    tx: mpsc::Sender<Arc<AcceptedMessage>>,
}

impl ChannelSink {
    /// Create a channel sink and its receiver with the given `capacity`.
    pub fn new(
        name: &'static str,
        capacity: usize,
    ) -> (Self, mpsc::Receiver<Arc<AcceptedMessage>>) {
        let (tx, rx) = mpsc::channel(capacity.max(1));
        (Self { name, tx }, rx)
    }
}

#[async_trait]
impl MessageSink for ChannelSink {
    fn name(&self) -> &'static str {
        self.name
    }

    async fn deliver(&self, message: Arc<AcceptedMessage>) -> Result<(), SinkError> {
        self.tx
            .send(message)
            .await
            .map_err(|_| SinkError::Closed(self.name))
    }
}

/// The acceptance pipeline: owns the auth policy, dedup cache and sinks.
pub struct Pipeline {
    /// Authorization policy consulted after signature verification.
    auth: Box<dyn AuthPolicy>,
    /// Bounded TTL duplicate-suppression cache.
    dedup: DedupCache,
    /// Fan-out targets for accepted messages.
    sinks: Vec<Box<dyn MessageSink>>,
}

impl Pipeline {
    /// Assemble a pipeline from its policy, dedup cache and sinks.
    pub fn new(
        auth: Box<dyn AuthPolicy>,
        dedup: DedupCache,
        sinks: Vec<Box<dyn MessageSink>>,
    ) -> Self {
        Self { auth, dedup, sinks }
    }

    /// Drain `rx` and process messages until the channel closes or `shutdown`
    /// fires.
    pub async fn run(mut self, mut rx: IngressReceiver, mut shutdown: ShutdownSignal) {
        tracing::info!(sinks = self.sinks.len(), "pipeline started");
        loop {
            tokio::select! {
                _ = shutdown.recv() => {
                    tracing::info!("pipeline shutting down");
                    break;
                }
                maybe = rx.recv() => {
                    match maybe {
                        Some(message) => self.process(message).await,
                        None => {
                            tracing::info!("ingress channel closed; pipeline stopping");
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Apply the canonical verify → authorize → deduplicate → fan-out sequence.
    async fn process(&mut self, message: IngressMessage) {
        let transport = message.transport.transport.as_str();

        let verified = match verify_envelope(&message.envelope) {
            Ok(verified) => verified,
            Err(err) => {
                metrics::counter!(VALIDATION_REJECTED_TOTAL, "transport" => transport).increment(1);
                tracing::debug!(%err, transport, "rejected: signature/structure invalid");
                return;
            }
        };

        if !self.auth.authorize(&verified.sensor_id) {
            metrics::counter!(AUTH_REJECTED_TOTAL, "transport" => transport).increment(1);
            tracing::debug!(sensor_id = %verified.sensor_id, transport, "rejected: not authorized");
            return;
        }

        let id = envelope_id(&message.raw_envelope);
        if !self.dedup.insert_if_new(id) {
            metrics::counter!(DEDUP_DROPPED_TOTAL, "transport" => transport).increment(1);
            tracing::debug!(envelope_id = %id, transport, "dropped: duplicate");
            return;
        }

        let accepted = Arc::new(AcceptedMessage {
            sensor_id: verified.sensor_id,
            envelope: verified.envelope,
            raw_envelope: message.raw_envelope,
            envelope_id: id,
            transport: message.transport,
            received_at: message.received_at,
        });

        tracing::info!(
            sensor_id = %accepted.sensor_id,
            envelope_id = %id,
            transport,
            "accepted envelope; publishing"
        );

        self.fan_out(accepted).await;
    }

    /// Deliver an accepted message to every sink, isolating per-sink failures.
    async fn fan_out(&self, message: Arc<AcceptedMessage>) {
        for sink in &self.sinks {
            if let Err(err) = sink.deliver(Arc::clone(&message)).await {
                tracing::warn!(sink = sink.name(), %err, "sink delivery failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::{NoneAuth, Whitelist};
    use crate::ingress::{channel, Transport};
    use crate::protocol::{encode_envelope, sign_message, SensorIdentity, SignOptions};
    use crate::shutdown::ShutdownController;
    use std::time::Duration;

    fn ingress_message(identity: &SensorIdentity, payload: &[u8]) -> IngressMessage {
        let envelope = sign_message(identity, payload, SignOptions::default());
        let raw = Bytes::from(encode_envelope(&envelope));
        IngressMessage {
            envelope,
            raw_envelope: raw,
            transport: TransportMetadata::http(None),
            received_at: SystemTime::now(),
        }
    }

    /// Build a message whose signature is invalid by corrupting a signed byte.
    fn tampered_message(identity: &SensorIdentity) -> IngressMessage {
        let mut message = ingress_message(identity, b"telemetry");
        message.envelope.message[0] ^= 0xff;
        message.raw_envelope = Bytes::from(encode_envelope(&message.envelope));
        message
    }

    fn dedup() -> DedupCache {
        DedupCache::new(64, Duration::from_secs(60))
    }

    async fn run_pipeline(
        auth: Box<dyn AuthPolicy>,
        messages: Vec<IngressMessage>,
    ) -> Vec<Arc<AcceptedMessage>> {
        let (sink, mut rx) = ChannelSink::new("test", 64);
        let pipeline = Pipeline::new(auth, dedup(), vec![Box::new(sink)]);

        let (tx, ingress_rx) = channel(64);
        let controller = ShutdownController::new();
        let shutdown = controller.subscribe();
        let handle = tokio::spawn(pipeline.run(ingress_rx, shutdown));

        for message in messages {
            tx.send(message).await.unwrap();
        }
        drop(tx); // Close the ingress channel so `run` returns.
        handle.await.unwrap();

        let mut delivered = Vec::new();
        while let Ok(message) = rx.try_recv() {
            delivered.push(message);
        }
        delivered
    }

    #[tokio::test]
    async fn valid_message_is_delivered() {
        let identity = SensorIdentity::from_secret_bytes(&[1u8; 32]);
        let delivered = run_pipeline(
            Box::new(NoneAuth),
            vec![ingress_message(&identity, b"hello")],
        )
        .await;
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].sensor_id, identity.sensor_id());
        assert_eq!(delivered[0].transport.transport, Transport::Http);
    }

    #[tokio::test]
    async fn invalid_signature_is_dropped() {
        let identity = SensorIdentity::from_secret_bytes(&[2u8; 32]);
        let delivered = run_pipeline(Box::new(NoneAuth), vec![tampered_message(&identity)]).await;
        assert!(delivered.is_empty());
    }

    #[tokio::test]
    async fn unauthorized_sensor_is_dropped() {
        let allowed = SensorIdentity::from_secret_bytes(&[3u8; 32]);
        let other = SensorIdentity::from_secret_bytes(&[4u8; 32]);
        let policy = Whitelist::new([allowed.sensor_id()]);

        let delivered = run_pipeline(
            Box::new(policy),
            vec![
                ingress_message(&allowed, b"ok"),
                ingress_message(&other, b"denied"),
            ],
        )
        .await;

        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].sensor_id, allowed.sensor_id());
    }

    #[tokio::test]
    async fn duplicate_message_is_delivered_once() {
        let identity = SensorIdentity::from_secret_bytes(&[5u8; 32]);
        // Same envelope bytes twice (fixed timestamp + nonce -> identical id).
        let envelope = sign_message(
            &identity,
            b"dup",
            SignOptions {
                timestamp_ms: Some(1_700_000_000_000),
                nonce: Some(vec![7u8; 16]),
            },
        );
        let raw = Bytes::from(encode_envelope(&envelope));
        let make = || IngressMessage {
            envelope: envelope.clone(),
            raw_envelope: raw.clone(),
            transport: TransportMetadata::http(None),
            received_at: SystemTime::now(),
        };

        let delivered = run_pipeline(Box::new(NoneAuth), vec![make(), make()]).await;
        assert_eq!(delivered.len(), 1);
    }

    #[tokio::test]
    async fn cross_transport_duplicate_is_dropped() {
        // The same signed envelope arriving over two different transports must
        // deduplicate, because the dedup key is the envelope bytes, not the
        // transport.
        let identity = SensorIdentity::from_secret_bytes(&[6u8; 32]);
        let envelope = sign_message(
            &identity,
            b"multi",
            SignOptions {
                timestamp_ms: Some(1_700_000_000_001),
                nonce: Some(vec![8u8; 16]),
            },
        );
        let raw = Bytes::from(encode_envelope(&envelope));

        let http = IngressMessage {
            envelope: envelope.clone(),
            raw_envelope: raw.clone(),
            transport: TransportMetadata {
                transport: Transport::Http,
                peer: None,
            },
            received_at: SystemTime::now(),
        };
        let mesh = IngressMessage {
            envelope: envelope.clone(),
            raw_envelope: raw.clone(),
            transport: TransportMetadata {
                transport: Transport::Meshtastic,
                peer: None,
            },
            received_at: SystemTime::now(),
        };

        let delivered = run_pipeline(Box::new(NoneAuth), vec![http, mesh]).await;
        assert_eq!(delivered.len(), 1);
    }
}
