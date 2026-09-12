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
//! HTTP ingress transport.
//!
//! Exposes a single endpoint, `POST /ingress`, that accepts a binary
//! Connectivity Protocol [`SignedEnvelope`] in the request body. The handler is
//! deliberately minimal:
//!
//! 1. A body-size limit ([`HttpConfig::max_body_bytes`]) rejects oversized
//!    requests with `413 Payload Too Large` before they are buffered.
//! 2. The body is structurally decoded into a [`SignedEnvelope`]; malformed
//!    bytes yield `400 Bad Request`.
//! 3. The resulting [`IngressMessage`] is offered to the bounded ingress channel
//!    with a non-blocking `try_send`, giving explicit backpressure:
//!    - `202 Accepted` when enqueued,
//!    - `429 Too Many Requests` when the pipeline is saturated (retryable),
//!    - `503 Service Unavailable` when the pipeline has shut down.
//!
//! No signature verification, authorization, deduplication or publishing happens
//! here; those are later pipeline stages keyed off the envelope `sensor_id`.

use super::{IngressMessage, IngressSender, Transport, TransportMetadata};
use crate::config::HttpConfig;
use crate::observability::metrics::{INGRESS_RECEIVED_TOTAL, VALIDATION_REJECTED_TOTAL};
use crate::protocol::decode_envelope;
use crate::shutdown::ShutdownSignal;
use async_trait::async_trait;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use std::time::SystemTime;
use tokio::sync::mpsc::error::TrySendError;

/// The single ingress route path.
const INGRESS_PATH: &str = "/telemetry/v1";

/// Shared handler state: the sending half of the bounded ingress channel.
#[derive(Clone)]
struct HttpState {
    /// Forwards accepted messages into the pipeline.
    tx: IngressSender,
}

/// HTTP ingress adapter.
///
/// Binds [`HttpConfig::listen`] and serves [`INGRESS_PATH`] until shutdown.
pub struct HttpIngress {
    /// Listener and limit configuration.
    config: HttpConfig,
}

impl HttpIngress {
    /// Create a new HTTP ingress from its configuration section.
    pub fn new(config: HttpConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl super::Ingress for HttpIngress {
    fn transport(&self) -> Transport {
        Transport::Http
    }

    async fn run(
        self: Box<Self>,
        tx: IngressSender,
        shutdown: ShutdownSignal,
    ) -> std::io::Result<()> {
        serve(&self.config, tx, shutdown).await
    }
}

/// Build the ingress router with a body-size limit and shared channel state.
///
/// Exposed for tests so the endpoint can be exercised without binding a socket.
pub fn router(tx: IngressSender, max_body_bytes: usize) -> Router {
    Router::new()
        .route(INGRESS_PATH, post(ingest))
        .layer(DefaultBodyLimit::max(max_body_bytes))
        .with_state(HttpState { tx })
}

/// Bind `config.listen` and serve ingress until `shutdown` fires.
///
/// Uses graceful shutdown so in-flight requests finish before the listener
/// closes.
pub async fn serve(
    config: &HttpConfig,
    tx: IngressSender,
    mut shutdown: ShutdownSignal,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    let bound = listener.local_addr()?;
    tracing::info!(address = %bound, path = INGRESS_PATH, "http ingress listening");

    axum::serve(listener, router(tx, config.max_body_bytes))
        .with_graceful_shutdown(async move {
            shutdown.recv().await;
            tracing::info!("http ingress shutting down");
        })
        .await
}

/// `POST /telemetry/v1` — accept a binary `SignedEnvelope` and enqueue it.
async fn ingest(State(state): State<HttpState>, body: Bytes) -> impl IntoResponse {
    let envelope = match decode_envelope(&body) {
        Ok(envelope) => envelope,
        Err(err) => {
            metrics::counter!(
                VALIDATION_REJECTED_TOTAL,
                "transport" => Transport::Http.as_str(),
                "stage" => "decode",
            )
            .increment(1);
            tracing::debug!(%err, "rejected malformed ingress body");
            return (StatusCode::BAD_REQUEST, format!("invalid envelope: {err}"));
        }
    };

    let message = IngressMessage {
        envelope,
        raw_envelope: body,
        transport: TransportMetadata::http(None),
        received_at: SystemTime::now(),
    };

    match state.tx.try_send(message) {
        Ok(()) => {
            metrics::counter!(
                INGRESS_RECEIVED_TOTAL,
                "transport" => Transport::Http.as_str(),
            )
            .increment(1);
            (StatusCode::ACCEPTED, "accepted".to_string())
        }
        Err(TrySendError::Full(_)) => {
            tracing::warn!("ingress channel full; applying backpressure");
            (
                StatusCode::TOO_MANY_REQUESTS,
                "ingress buffer full".to_string(),
            )
        }
        Err(TrySendError::Closed(_)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "ingress pipeline unavailable".to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingress::channel;
    use crate::protocol::{encode_envelope, sign_message, SensorIdentity, SignOptions};
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// Build a valid, signed envelope's wire bytes for request bodies.
    fn valid_envelope_bytes() -> Vec<u8> {
        let identity = SensorIdentity::from_secret_bytes(&[9u8; 32]);
        let envelope = sign_message(&identity, b"telemetry", SignOptions::default());
        encode_envelope(&envelope)
    }

    /// Issue a `POST /ingress` with the given body against a router.
    async fn post_ingress(router: Router, body: Vec<u8>) -> StatusCode {
        router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(INGRESS_PATH)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn valid_envelope_is_accepted_and_forwarded() {
        let (tx, mut rx) = channel(4);
        let status = post_ingress(router(tx, 64 * 1024), valid_envelope_bytes()).await;
        assert_eq!(status, StatusCode::ACCEPTED);

        let message = rx.try_recv().expect("message forwarded to pipeline");
        assert_eq!(message.transport.transport, Transport::Http);
        assert_eq!(message.envelope.message, b"telemetry");
        assert!(!message.raw_envelope.is_empty());
    }

    #[tokio::test]
    async fn malformed_body_is_rejected() {
        let (tx, _rx) = channel(4);
        let status = post_ingress(router(tx, 64 * 1024), vec![0xff, 0xff, 0xff]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn oversized_body_is_rejected() {
        let (tx, _rx) = channel(4);
        // Limit of 8 bytes; a valid envelope is far larger.
        let status = post_ingress(router(tx, 8), valid_envelope_bytes()).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn full_channel_yields_backpressure() {
        let (tx, _rx) = channel(1);
        // Fill the single slot; the receiver is never drained.
        let first = post_ingress(router(tx.clone(), 64 * 1024), valid_envelope_bytes()).await;
        assert_eq!(first, StatusCode::ACCEPTED);

        let second = post_ingress(router(tx, 64 * 1024), valid_envelope_bytes()).await;
        assert_eq!(second, StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn closed_pipeline_is_unavailable() {
        let (tx, rx) = channel(4);
        drop(rx);
        let status = post_ingress(router(tx, 64 * 1024), valid_envelope_bytes()).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    }
}
