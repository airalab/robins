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
//! Ingress transports and the canonical message they all produce.
//!
//! The protocol is canonical and transports are adapters: every transport
//! (HTTP today; Meshtastic later) decodes its wire format into the single
//! [`IngressMessage`] type and hands it to the shared validation/auth pipeline
//! over a **bounded** channel. Nothing about the transport ever influences
//! authorization — that is decided solely from the verified envelope
//! `sensor_id` downstream.
//!
//! An ingress adapter is intentionally *dumb*: it decodes structurally, applies
//! resource limits (body size, backpressure) and forwards. It performs no
//! signature verification, deduplication, authorization or publishing; those
//! live in later pipeline stages.

pub mod http;

use crate::protocol::SignedEnvelope;
use crate::shutdown::ShutdownSignal;
use async_trait::async_trait;
use bytes::Bytes;
use std::time::SystemTime;
use tokio::sync::mpsc;

/// The ingress transport that accepted a given [`IngressMessage`].
///
/// This is diagnostic metadata only; it must never affect validation or
/// authorization decisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// HTTP `POST` ingress.
    Http,
    /// Meshtastic radio ingress (reserved for a later phase).
    Meshtastic,
}

impl Transport {
    /// A stable, lowercase label suitable for metric dimensions and logs.
    pub fn as_str(&self) -> &'static str {
        match self {
            Transport::Http => "http",
            Transport::Meshtastic => "meshtastic",
        }
    }
}

/// Non-authoritative context describing how a message arrived.
#[derive(Clone, Debug)]
pub struct TransportMetadata {
    /// Which transport accepted the message.
    pub transport: Transport,
    /// Best-effort peer identifier (e.g. remote socket address), if known.
    pub peer: Option<String>,
}

impl TransportMetadata {
    /// Construct metadata for the HTTP transport.
    pub fn http(peer: Option<String>) -> Self {
        Self {
            transport: Transport::Http,
            peer,
        }
    }
}

/// The canonical unit of work handed from any ingress transport to the pipeline.
///
/// It carries both the decoded [`SignedEnvelope`] and the **exact** wire bytes
/// it was decoded from. Downstream stages must use [`raw_envelope`] for the
/// dedup/message id and for re-publishing, never a re-encoding of `envelope`:
/// protobuf serialization is not canonical, so a round-trip may differ from the
/// signed bytes.
///
/// [`raw_envelope`]: IngressMessage::raw_envelope
#[derive(Clone, Debug)]
pub struct IngressMessage {
    /// The structurally decoded envelope (signature **not** yet verified).
    pub envelope: SignedEnvelope,
    /// The exact bytes the envelope was decoded from.
    pub raw_envelope: Bytes,
    /// How the message arrived (diagnostic only).
    pub transport: TransportMetadata,
    /// Wall-clock time the message was accepted at the ingress boundary.
    pub received_at: SystemTime,
}

/// Sending half of the bounded ingress channel.
///
/// The channel is bounded so a slow pipeline exerts backpressure on transports
/// instead of growing memory without limit. Transports translate a full channel
/// into a transport-appropriate rejection (e.g. HTTP `429`).
pub type IngressSender = mpsc::Sender<IngressMessage>;

/// Receiving half of the bounded ingress channel, consumed by the pipeline.
pub type IngressReceiver = mpsc::Receiver<IngressMessage>;

/// Create the bounded ingress channel with the configured capacity.
///
/// `capacity` comes from `[pipeline] ingress_buffer` and is clamped to at least
/// one slot so a zero in configuration cannot deadlock the daemon.
pub fn channel(capacity: usize) -> (IngressSender, IngressReceiver) {
    mpsc::channel(capacity.max(1))
}

/// A runnable ingress transport.
///
/// Implementors own their listener/device and run until `shutdown` fires,
/// forwarding every accepted [`IngressMessage`] to `tx`. `run` consumes the
/// adapter so its resources are released on return.
#[async_trait]
pub trait Ingress: Send {
    /// The transport this adapter implements.
    fn transport(&self) -> Transport;

    /// Serve the transport until `shutdown` is signalled.
    async fn run(
        self: Box<Self>,
        tx: IngressSender,
        shutdown: ShutdownSignal,
    ) -> std::io::Result<()>;
}
