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
//! The operations HTTP server: `/health`, `/ready` and `/metrics`.
//!
//! This is an internal, unauthenticated endpoint intended to be bound to a
//! private address (see `[metrics] listen`). It shares nothing with the ingress
//! transport and never touches envelope data.

use super::health::Health;
use crate::p2p::PeerRegistry;
use crate::shutdown::ShutdownSignal;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use metrics_exporter_prometheus::PrometheusHandle;
use std::net::SocketAddr;

/// Shared state made available to the operations handlers.
#[derive(Clone)]
struct OpsState {
    /// Liveness/readiness flags.
    health: Health,
    /// Prometheus exposition renderer.
    metrics: PrometheusHandle,
    /// Optional connected-peer registry backing `/debug/peers`.
    peers: Option<PeerRegistry>,
}

/// Build the operations router without peer diagnostics.
///
/// Exposed so it can be exercised directly in tests without binding a socket.
pub fn router(health: Health, metrics: PrometheusHandle) -> Router {
    build_router(health, metrics, None)
}

/// Build the operations router including the `/debug/peers` provider.
pub fn router_with_peers(health: Health, metrics: PrometheusHandle, peers: PeerRegistry) -> Router {
    build_router(health, metrics, Some(peers))
}

/// Assemble the operations router with an optional peer registry.
fn build_router(health: Health, metrics: PrometheusHandle, peers: Option<PeerRegistry>) -> Router {
    Router::new()
        .route("/health", get(health_handler))
        .route("/ready", get(ready_handler))
        .route("/metrics", get(metrics_handler))
        .route("/debug/peers", get(peers_handler))
        .with_state(OpsState {
            health,
            metrics,
            peers,
        })
}

/// Bind `listen` and serve the operations endpoints until `shutdown` fires.
///
/// Uses graceful shutdown so in-flight scrapes complete before the server
/// stops.
pub async fn serve(
    listen: SocketAddr,
    health: Health,
    metrics: PrometheusHandle,
    peers: Option<PeerRegistry>,
    mut shutdown: ShutdownSignal,
) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    let bound = listener.local_addr()?;
    tracing::info!(address = %bound, "operations server listening");

    axum::serve(listener, build_router(health, metrics, peers))
        .with_graceful_shutdown(async move {
            shutdown.recv().await;
            tracing::info!("operations server shutting down");
        })
        .await
}

/// `GET /health` — liveness probe. `200` while the process is live.
async fn health_handler(State(state): State<OpsState>) -> impl IntoResponse {
    if state.health.is_live() {
        (StatusCode::OK, "ok")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "unhealthy")
    }
}

/// `GET /ready` — readiness probe. `200` once ready, else `503`.
async fn ready_handler(State(state): State<OpsState>) -> impl IntoResponse {
    if state.health.is_ready() {
        (StatusCode::OK, "ready")
    } else {
        (StatusCode::SERVICE_UNAVAILABLE, "not ready")
    }
}

/// `GET /metrics` — Prometheus exposition of the installed recorder.
async fn metrics_handler(State(state): State<OpsState>) -> impl IntoResponse {
    let body = state.metrics.render();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

/// `GET /debug/peers` — JSON array of currently connected libp2p peers.
///
/// Returns an empty array when no GossipSub node is attached.
async fn peers_handler(State(state): State<OpsState>) -> impl IntoResponse {
    let peers = state
        .peers
        .map(|registry| registry.snapshot())
        .unwrap_or_default();
    (StatusCode::OK, Json(peers))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::metrics::test_handle;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn get(router: Router, uri: &str) -> (StatusCode, String) {
        let response = router
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn health_is_ok() {
        let (status, body) = get(router(Health::new(), test_handle()), "/health").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
    }

    #[tokio::test]
    async fn ready_reflects_state() {
        let health = Health::new();
        let (status, _) = get(router(health.clone(), test_handle()), "/ready").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        health.set_ready(true);
        let (status, body) = get(router(health, test_handle()), "/ready").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ready");
    }

    #[tokio::test]
    async fn metrics_renders() {
        let (status, _body) = get(router(Health::new(), test_handle()), "/metrics").await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn debug_peers_reports_registry() {
        use crate::p2p::PeerRegistry;
        // Without a registry the endpoint returns an empty array.
        let (status, body) = get(router(Health::new(), test_handle()), "/debug/peers").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "[]");

        // With a registry the connected peer is reported.
        let registry = PeerRegistry::new();
        let peer = libp2p::identity::Keypair::generate_ed25519()
            .public()
            .to_peer_id();
        registry.on_connected(peer, None);
        let router = router_with_peers(Health::new(), test_handle(), registry);
        let (status, body) = get(router, "/debug/peers").await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("peer_id"));
    }
}
