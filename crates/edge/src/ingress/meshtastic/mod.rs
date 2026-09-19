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
//! Meshtastic radio ingress transport.
//!
//! Connects to a Meshtastic radio over USB serial and consumes Connectivity
//! Protocol Transport v1 frames (`src/protobufs/transport/meshtastic/v1.md`)
//! addressed to the configured application `PortNum`. Frame parsing
//! ([`frame`]) and bounded fragment reassembly ([`reassembly`]) are pure,
//! synchronous, and independently unit-tested; this module only supplies the
//! async serial I/O, reconnect loop, and translation into the shared
//! [`IngressMessage`] pipeline type.
//!
//! Like HTTP ingress, this adapter is a narrow transport shim: it performs no
//! signature verification, authorization, or deduplication. Meshtastic PKI
//! authentication (`MeshPacket.pki_encrypted`) is verified because the
//! transport spec makes it a hard requirement (§13/§14/§27), but this is
//! still transport-level: it says nothing about upper-layer
//! (`SignedEnvelope`) authenticity.

pub mod frame;
pub mod reassembly;

pub use frame::MAX_FRAGMENT_COUNT;

use super::{Ingress, IngressMessage, IngressSender, Transport, TransportMetadata};
use crate::config::MeshtasticConfig;
use crate::observability::metrics::{
    INGRESS_RECEIVED_TOTAL, MESHTASTIC_CONNECTION_UP, MESHTASTIC_FRAMES_INVALID_TOTAL,
    MESHTASTIC_PACKETS_IGNORED_TOTAL, MESHTASTIC_PACKETS_RECEIVED_TOTAL,
    MESHTASTIC_REASSEMBLY_COMPLETED_TOTAL, MESHTASTIC_REASSEMBLY_CONFLICT_TOTAL,
    MESHTASTIC_REASSEMBLY_EXPIRED_TOTAL, MESHTASTIC_REASSEMBLY_OVERSIZE_TOTAL,
    MESHTASTIC_REASSEMBLY_PENDING, MESHTASTIC_REASSEMBLY_STARTED_TOTAL,
    MESHTASTIC_RECONNECTS_TOTAL, VALIDATION_REJECTED_TOTAL,
};
use crate::protocol::{decode_envelope, SensorId};
use crate::shutdown::ShutdownSignal;
use async_trait::async_trait;
use bytes::Bytes;
use frame::Frame;
use meshtastic::api::StreamApi;
use meshtastic::packet::PacketReceiver;
use meshtastic::protobufs::{from_radio, mesh_packet, FromRadio};
use meshtastic::utils;
use reassembly::{Outcome, Reassembler, ReassemblerConfig};
use std::time::{Duration, Instant, SystemTime};

/// Meshtastic serial ingress adapter.
pub struct MeshtasticIngress {
    config: MeshtasticConfig,
}

impl MeshtasticIngress {
    /// Create a new Meshtastic ingress from its configuration section.
    ///
    /// Callers must only construct this when `config.enabled` and
    /// `Config::validate` has already accepted the configuration (transport
    /// == `"serial"`, `device` present, numeric limits valid).
    pub fn new(config: MeshtasticConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Ingress for MeshtasticIngress {
    fn transport(&self) -> Transport {
        Transport::Meshtastic
    }

    async fn run(
        self: Box<Self>,
        tx: IngressSender,
        shutdown: ShutdownSignal,
    ) -> std::io::Result<()> {
        run_reconnect_loop(self.config, tx, shutdown).await;
        Ok(())
    }
}

impl From<&MeshtasticConfig> for ReassemblerConfig {
    fn from(config: &MeshtasticConfig) -> Self {
        Self {
            max_reassembled_bytes: config.max_reassembled_bytes,
            max_fragments: config.max_fragments,
            max_pending: config.max_pending,
            max_pending_per_sender: config.max_pending_per_sender,
            idle_timeout: Duration::from_secs(config.reassembly_timeout_secs),
            absolute_timeout: Duration::from_secs(config.reassembly_absolute_timeout_secs),
        }
    }
}

/// Owns the reconnect loop: repeatedly open the serial session, serve
/// packets until disconnect/error, and back off before retrying. Runs until
/// `shutdown` fires.
async fn run_reconnect_loop(
    config: MeshtasticConfig,
    tx: IngressSender,
    mut shutdown: ShutdownSignal,
) {
    let min_backoff = Duration::from_secs(config.reconnect_min_secs.max(1));
    let max_backoff = Duration::from_secs(
        config
            .reconnect_max_secs
            .max(config.reconnect_min_secs.max(1)),
    );
    let mut backoff = min_backoff;

    loop {
        if shutdown.is_triggered() {
            return;
        }

        let (ended, connected) = connect_and_serve(&config, tx.clone(), shutdown.clone()).await;
        if connected {
            // A successful connection resets the backoff.
            backoff = min_backoff;
        }

        match ended {
            SessionEnd::Shutdown => return,
            SessionEnd::Disconnected => {
                metrics::counter!(MESHTASTIC_RECONNECTS_TOTAL).increment(1);
                tracing::warn!(
                    device = config.device.as_deref().unwrap_or(""),
                    backoff_secs = backoff.as_secs(),
                    "meshtastic serial session ended; reconnecting"
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown.recv() => return,
                }
                backoff = (backoff * 2).min(max_backoff);
            }
        }
    }
}

/// Why a serial session loop returned.
enum SessionEnd {
    /// Shutdown was requested; the caller must not reconnect.
    Shutdown,
    /// The serial connection failed to open, or an open session ended
    /// (disconnect, radio error, or the decoded-packet channel closed).
    Disconnected,
}

/// Open one serial session, serve packets from it until disconnect or
/// shutdown, and tear it down. Returns why the session ended and whether it
/// ever reached the `Configured` state (used to reset reconnect backoff).
async fn connect_and_serve(
    config: &MeshtasticConfig,
    tx: IngressSender,
    mut shutdown: ShutdownSignal,
) -> (SessionEnd, bool) {
    let device = match &config.device {
        Some(device) => device.clone(),
        None => {
            tracing::error!("meshtastic.enabled = true but meshtastic.device is unset");
            return (SessionEnd::Disconnected, false);
        }
    };

    let stream_handle = match utils::stream::build_serial_stream(device.clone(), None, None, None) {
        Ok(handle) => handle,
        Err(err) => {
            tracing::warn!(%err, device, "failed to open meshtastic serial device");
            return (SessionEnd::Disconnected, false);
        }
    };

    let stream_api = StreamApi::new();
    let (decoded_rx, stream_api) = stream_api.connect(stream_handle).await;
    let config_id = utils::generate_rand_id();
    let stream_api = match stream_api.configure(config_id).await {
        Ok(api) => api,
        Err(err) => {
            tracing::warn!(%err, device, "failed to configure meshtastic session");
            return (SessionEnd::Disconnected, false);
        }
    };

    metrics::gauge!(MESHTASTIC_CONNECTION_UP).set(1.0);
    tracing::info!(device, "meshtastic serial session established");

    let ended = serve_session(config, tx, decoded_rx, &mut shutdown).await;
    metrics::gauge!(MESHTASTIC_REASSEMBLY_PENDING).set(0.0);

    metrics::gauge!(MESHTASTIC_CONNECTION_UP).set(0.0);
    let _ = stream_api.disconnect().await;
    tracing::info!(device, "meshtastic serial session closed");

    (ended, true)
}

/// How often incomplete reassemblies are swept for idle/absolute expiry.
const REASSEMBLY_SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Serve decoded `FromRadio` packets until the channel closes (disconnect)
/// or shutdown fires. Reassembly state lives entirely within this function's
/// scope, so it is naturally cleared when the session ends.
async fn serve_session(
    config: &MeshtasticConfig,
    tx: IngressSender,
    mut decoded_rx: PacketReceiver,
    shutdown: &mut ShutdownSignal,
) -> SessionEnd {
    let mut reassembler = Reassembler::new(ReassemblerConfig::from(config));
    let mut sweep = tokio::time::interval(REASSEMBLY_SWEEP_INTERVAL);

    loop {
        tokio::select! {
            _ = shutdown.recv() => return SessionEnd::Shutdown,
            _ = sweep.tick() => {
                let evicted = reassembler.sweep_expired(Instant::now());
                if evicted > 0 {
                    metrics::counter!(MESHTASTIC_REASSEMBLY_EXPIRED_TOTAL).increment(evicted as u64);
                }
                metrics::gauge!(MESHTASTIC_REASSEMBLY_PENDING).set(reassembler.pending_len() as f64);
            }
            packet = decoded_rx.recv() => {
                let Some(packet) = packet else {
                    // The decoded-packet channel closes when the read task
                    // ends, i.e. the serial connection was lost.
                    return SessionEnd::Disconnected;
                };
                if let Some(message) = handle_packet(config, &mut reassembler, packet) {
                    metrics::gauge!(MESHTASTIC_REASSEMBLY_PENDING).set(reassembler.pending_len() as f64);
                    tokio::select! {
                        _ = shutdown.recv() => return SessionEnd::Shutdown,
                        send_result = tx.send(message) => {
                            if send_result.is_err() {
                                // Pipeline shut down; treat like a shutdown request.
                                return SessionEnd::Shutdown;
                            }
                        }
                    }
                } else {
                    metrics::gauge!(MESHTASTIC_REASSEMBLY_PENDING).set(reassembler.pending_len() as f64);
                }
            }
        }
    }
}

/// Process one `FromRadio` message: filter, parse, reassemble, and
/// structurally decode. Returns `Some(IngressMessage)` when a complete,
/// structurally valid envelope is ready to submit; otherwise `None`
/// (progress, ignored, invalid, or rejected — all reflected in metrics/logs).
fn handle_packet(
    config: &MeshtasticConfig,
    reassembler: &mut Reassembler,
    packet: FromRadio,
) -> Option<IngressMessage> {
    let Some(from_radio::PayloadVariant::Packet(mesh_packet)) = packet.payload_variant else {
        return None;
    };

    let Some(mesh_packet::PayloadVariant::Decoded(data)) = mesh_packet.payload_variant.clone()
    else {
        return None;
    };

    if data.portnum != config.port_num as i32 {
        return None;
    }

    metrics::counter!(MESHTASTIC_PACKETS_RECEIVED_TOTAL).increment(1);

    if !mesh_packet.pki_encrypted {
        metrics::counter!(MESHTASTIC_PACKETS_IGNORED_TOTAL).increment(1);
        tracing::warn!(
            mesh_sender = format_args!("{:08x}", mesh_packet.from),
            "ignoring meshtastic packet without successful PKI authentication"
        );
        return None;
    }

    let mesh_sender = mesh_packet.from;
    let payload = Bytes::from(data.payload);

    let raw_envelope = match frame::parse_frame(payload) {
        Ok(Frame::Single(bytes)) => bytes,
        Ok(Frame::Fragment(fragment)) => {
            match reassembler.accept(Instant::now(), mesh_sender, fragment) {
                Outcome::Progress => {
                    metrics::counter!(MESHTASTIC_REASSEMBLY_STARTED_TOTAL).increment(1);
                    return None;
                }
                Outcome::DuplicateIgnored => return None,
                Outcome::Conflict => {
                    metrics::counter!(MESHTASTIC_REASSEMBLY_CONFLICT_TOTAL).increment(1);
                    tracing::warn!(
                        mesh_sender = format_args!("{:08x}", mesh_sender),
                        "meshtastic fragment conflict; assembly discarded"
                    );
                    return None;
                }
                Outcome::Rejected(reason) => {
                    metrics::counter!(MESHTASTIC_REASSEMBLY_OVERSIZE_TOTAL).increment(1);
                    tracing::warn!(
                        mesh_sender = format_args!("{:08x}", mesh_sender),
                        ?reason,
                        "meshtastic reassembly rejected"
                    );
                    return None;
                }
                Outcome::Complete(bytes) => {
                    metrics::counter!(MESHTASTIC_REASSEMBLY_COMPLETED_TOTAL).increment(1);
                    bytes
                }
            }
        }
        Err(err) => {
            metrics::counter!(MESHTASTIC_FRAMES_INVALID_TOTAL).increment(1);
            tracing::warn!(
                mesh_sender = format_args!("{:08x}", mesh_sender),
                %err,
                "invalid meshtastic transport frame"
            );
            return None;
        }
    };

    let envelope = match decode_envelope(&raw_envelope) {
        Ok(envelope) => envelope,
        Err(err) => {
            metrics::counter!(
                VALIDATION_REJECTED_TOTAL,
                "transport" => Transport::Meshtastic.as_str(),
                "stage" => "decode",
            )
            .increment(1);
            tracing::warn!(
                mesh_sender = format_args!("{:08x}", mesh_sender),
                %err,
                "meshtastic envelope failed structural decode"
            );
            return None;
        }
    };

    let sensor_id = SensorId::from_slice(&envelope.sensor_id).ok();
    metrics::counter!(
        INGRESS_RECEIVED_TOTAL,
        "transport" => Transport::Meshtastic.as_str(),
    )
    .increment(1);
    match sensor_id {
        Some(sensor_id) => tracing::info!(
            %sensor_id,
            mesh_sender = format_args!("{:08x}", mesh_sender),
            "accepted telemetry envelope over meshtastic"
        ),
        None => tracing::info!(
            mesh_sender = format_args!("{:08x}", mesh_sender),
            "accepted telemetry envelope over meshtastic (unparsable sensor id)"
        ),
    }

    Some(IngressMessage {
        envelope,
        raw_envelope,
        transport: TransportMetadata::meshtastic(mesh_sender),
        received_at: SystemTime::now(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MeshtasticConfig;
    use crate::protocol::{encode_envelope, sign_message, SensorIdentity, SignOptions};
    use meshtastic::protobufs::{Data, MeshPacket};

    fn test_config() -> MeshtasticConfig {
        MeshtasticConfig {
            enabled: true,
            transport: Some("serial".to_string()),
            device: Some("/dev/null".to_string()),
            port_num: 256,
            max_reassembled_bytes: 4096,
            max_fragments: 16,
            max_pending: 128,
            max_pending_per_sender: 8,
            reassembly_timeout_secs: 60,
            reassembly_absolute_timeout_secs: 300,
            reconnect_min_secs: 1,
            reconnect_max_secs: 30,
        }
    }

    fn valid_envelope_bytes() -> Vec<u8> {
        let identity = SensorIdentity::from_secret_bytes(&[3u8; 32]);
        let envelope = sign_message(&identity, b"telemetry", SignOptions::default());
        encode_envelope(&envelope)
    }

    /// A signed envelope large enough (> `MAX_FRAGMENT_BODY` bytes) that it
    /// always requires at least two Transport v1 fragments, for tests that
    /// exercise the reassembly path.
    fn large_envelope_bytes() -> Vec<u8> {
        let identity = SensorIdentity::from_secret_bytes(&[3u8; 32]);
        let payload = vec![0x42u8; 512];
        let envelope = sign_message(&identity, &payload, SignOptions::default());
        encode_envelope(&envelope)
    }

    fn single_frame_packet(mesh_sender: u32, port_num: i32, pki: bool, raw: &[u8]) -> FromRadio {
        let mut payload = vec![frame::SINGLE_CONTROL];
        payload.extend_from_slice(raw);
        let data = Data {
            portnum: port_num,
            payload,
            ..Default::default()
        };
        let mesh_packet = MeshPacket {
            from: mesh_sender,
            pki_encrypted: pki,
            payload_variant: Some(mesh_packet::PayloadVariant::Decoded(data)),
            ..Default::default()
        };
        FromRadio {
            payload_variant: Some(from_radio::PayloadVariant::Packet(mesh_packet)),
            ..Default::default()
        }
    }

    #[test]
    fn unrelated_port_num_is_ignored() {
        let config = test_config();
        let mut reassembler = Reassembler::new(ReassemblerConfig::from(&config));
        let packet = single_frame_packet(1, 1 /* TextMessageApp */, true, b"hello");
        assert!(handle_packet(&config, &mut reassembler, packet).is_none());
    }

    #[test]
    fn non_pki_packet_is_ignored() {
        let config = test_config();
        let mut reassembler = Reassembler::new(ReassemblerConfig::from(&config));
        let bytes = valid_envelope_bytes();
        let packet = single_frame_packet(1, 256, false, &bytes);
        assert!(handle_packet(&config, &mut reassembler, packet).is_none());
    }

    #[test]
    fn valid_single_frame_envelope_is_accepted() {
        let config = test_config();
        let mut reassembler = Reassembler::new(ReassemblerConfig::from(&config));
        let bytes = valid_envelope_bytes();
        let packet = single_frame_packet(0xdeadbeef, 256, true, &bytes);

        let message = handle_packet(&config, &mut reassembler, packet).expect("accepted");
        assert_eq!(message.transport.transport, Transport::Meshtastic);
        assert_eq!(message.transport.peer.as_deref(), Some("!deadbeef"));
        assert_eq!(message.raw_envelope.as_ref(), bytes.as_slice());
        assert_eq!(message.envelope.message, b"telemetry");
    }

    #[test]
    fn malformed_envelope_never_reaches_the_pipeline() {
        let config = test_config();
        let mut reassembler = Reassembler::new(ReassemblerConfig::from(&config));
        let packet = single_frame_packet(1, 256, true, &[0xff, 0xff, 0xff]);
        assert!(handle_packet(&config, &mut reassembler, packet).is_none());
    }

    #[test]
    fn fragmented_envelope_is_reassembled_and_accepted() {
        let config = test_config();
        let mut reassembler = Reassembler::new(ReassemblerConfig::from(&config));
        // Use a payload large enough to guarantee multiple wire fragments,
        // then drive the same frame parser/reassembler path `handle_packet`
        // uses, in spec-conformant (exactly `MAX_FRAGMENT_BODY`-sized,
        // except possibly the last) chunks.
        let bytes = large_envelope_bytes();
        let id = {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(&bytes);
            let mut id = [0u8; 6];
            id.copy_from_slice(&digest[..6]);
            id
        };

        let chunks: Vec<&[u8]> = bytes.chunks(frame::MAX_FRAGMENT_BODY).collect();
        let count = chunks.len() as u8;
        assert!(count >= 2, "test envelope must require fragmentation");
        let mut last = None;
        for (index, chunk) in chunks.iter().enumerate() {
            let f = frame::FragmentFrame {
                message_id: id,
                index: index as u8,
                count,
                body: Bytes::from(chunk.to_vec()),
            };
            last = Some(reassembler.accept(Instant::now(), 42, f));
        }
        match last {
            Some(Outcome::Complete(reassembled)) => {
                assert_eq!(reassembled.as_ref(), bytes.as_slice());
            }
            other => panic!("expected completion, got {other:?}"),
        }
    }
}
