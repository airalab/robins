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
//! Top-level daemon wiring: assemble subsystems, start them, shut them down.
//!
//! [`EdgeApp::start`] wires the MVP data path in dependency order:
//!
//! ```text
//!   HTTP ingress ─┐
//!                 ├─▶ bounded channel ─▶ pipeline (verify→auth→dedup) ─▶ gossip
//!   (future mesh)─┘                                                       publisher
//!   ops server (/health,/ready,/metrics,/debug/peers)
//! ```
//!
//! Listeners are bound eagerly so their concrete addresses are known before the
//! serving tasks spawn (useful for `:0` ephemeral ports in tests). A single
//! [`ShutdownController`] fans a shutdown signal out to every task; [`shutdown`]
//! triggers it and awaits graceful termination.
//!
//! [`shutdown`]: EdgeApp::shutdown

use crate::auth;
use crate::config::Config;
use crate::ingress::{self, IngressSender};
use crate::observability::{self, Health};
use crate::p2p::{self, GossipConfig, GossipNode, PeerRegistry};
use crate::pipeline::{ChannelSink, DedupCache, MessageSink, Pipeline};
use crate::shutdown::ShutdownController;
use anyhow::Context;
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

/// Capacity of the pipeline → gossip publisher channel.
const GOSSIP_SINK_BUFFER: usize = 1024;
/// How long to wait for the gossip node to report its first listen address.
const GOSSIP_LISTEN_TIMEOUT: Duration = Duration::from_secs(5);

/// A running gateway: bound addresses plus the task handles that serve them.
pub struct EdgeApp {
    /// Bound HTTP ingress address, if the HTTP transport is enabled.
    pub http_addr: Option<SocketAddr>,
    /// Bound operations-server address.
    pub ops_addr: SocketAddr,
    /// First bound GossipSub listen address, if pubsub is enabled.
    pub gossip_listen: Option<libp2p::Multiaddr>,
    /// The stable local libp2p peer id, if pubsub is enabled.
    pub gossip_peer_id: Option<libp2p::PeerId>,
    /// Controller used to signal graceful shutdown to every task.
    controller: ShutdownController,
    /// Spawned serving tasks.
    tasks: Vec<JoinHandle<()>>,
}

impl EdgeApp {
    /// Assemble and start every enabled subsystem from `config`.
    ///
    /// Returns once all listeners are bound and tasks are spawned. Observability
    /// (tracing + the Prometheus recorder) is initialised as a side effect; a
    /// second initialisation in the same process is ignored.
    pub async fn start(config: Config) -> anyhow::Result<Self> {
        observability::init_tracing("info");
        let metrics = install_metrics();
        let health = Health::new();
        let controller = ShutdownController::new();
        let mut tasks = Vec::new();

        // Ingress → pipeline channel (bounded for backpressure).
        let (ingress_tx, ingress_rx) = ingress::channel(config.pipeline.ingress_buffer);

        // GossipSub publisher and the sink the pipeline fans out to.
        let peers = PeerRegistry::new();
        let mut sinks: Vec<Box<dyn MessageSink>> = Vec::new();
        let mut gossip_listen = None;
        let mut gossip_peer_id = None;

        if config.pubsub.enabled {
            let (sink, publish_rx) = ChannelSink::new("gossip", GOSSIP_SINK_BUFFER);
            sinks.push(Box::new(sink));

            let gossip_config = GossipConfig::from_pubsub(&config.pubsub)
                .context("invalid [pubsub] configuration")?;
            let keypair = p2p::load_or_create_identity(config.pubsub.identity_file.as_deref())
                .context("failed to load gossip identity")?;
            let node = GossipNode::new(
                keypair,
                gossip_config,
                peers.clone(),
                Some(health.clone()),
                None,
            )
            .await
            .context("failed to build gossip node")?;
            gossip_peer_id = Some(node.local_peer_id());

            let (addr_tx, mut addr_rx) = mpsc::channel(4);
            let shutdown = controller.subscribe();
            tasks.push(tokio::spawn(async move {
                node.run(publish_rx, shutdown, Some(addr_tx)).await;
            }));
            gossip_listen = tokio::time::timeout(GOSSIP_LISTEN_TIMEOUT, addr_rx.recv())
                .await
                .ok()
                .flatten();
        } else {
            // No peer gating: the gateway is immediately ready.
            health.set_ready(true);
        }

        // Acceptance pipeline.
        let policy = auth::from_config(&config.auth).context("failed to build auth policy")?;
        let dedup = DedupCache::new(
            config.pipeline.dedup_capacity,
            Duration::from_secs(config.pipeline.dedup_ttl_secs),
        );
        let pipeline = Pipeline::new(policy, dedup, sinks);
        let pipeline_shutdown = controller.subscribe();
        tasks.push(tokio::spawn(pipeline.run(ingress_rx, pipeline_shutdown)));

        // HTTP ingress.
        let http_addr = if config.http.enabled {
            Some(
                spawn_http_ingress(&config, ingress_tx, &controller, &mut tasks)
                    .await
                    .context("failed to start HTTP ingress")?,
            )
        } else {
            None
        };

        // Operations server.
        let ops_addr = spawn_ops_server(&config, health, metrics, peers, &controller, &mut tasks)
            .await
            .context("failed to start operations server")?;

        tracing::info!(
            ?http_addr,
            %ops_addr,
            ?gossip_listen,
            "edge gateway started"
        );

        Ok(Self {
            http_addr,
            ops_addr,
            gossip_listen,
            gossip_peer_id,
            controller,
            tasks,
        })
    }

    /// Signal shutdown and await graceful termination of every task.
    pub async fn shutdown(self) {
        self.controller.trigger();
        for task in self.tasks {
            let _ = task.await;
        }
    }
}

/// Run the gateway until a `Ctrl-C` (SIGINT) is received, then shut down.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let app = EdgeApp::start(config).await?;
    tokio::signal::ctrl_c()
        .await
        .context("failed to listen for shutdown signal")?;
    tracing::info!("shutdown signal received");
    app.shutdown().await;
    Ok(())
}

/// Install the global Prometheus recorder, tolerating a prior installation.
///
/// The recorder is process-global, so a second call (e.g. a restart within the
/// same test process) is expected to fail; we log and continue with a
/// non-recording handle rather than aborting startup.
fn install_metrics() -> metrics_exporter_prometheus::PrometheusHandle {
    match observability::metrics::install() {
        Ok(handle) => handle,
        Err(err) => {
            tracing::warn!(%err, "metrics recorder already installed; continuing");
            metrics_exporter_prometheus::PrometheusBuilder::new()
                .build_recorder()
                .handle()
        }
    }
}

/// Bind and serve HTTP ingress, returning the bound address.
async fn spawn_http_ingress(
    config: &Config,
    ingress_tx: IngressSender,
    controller: &ShutdownController,
    tasks: &mut Vec<JoinHandle<()>>,
) -> anyhow::Result<SocketAddr> {
    let listener = TcpListener::bind(config.http.listen)
        .await
        .with_context(|| format!("failed to bind HTTP ingress on {}", config.http.listen))?;
    let addr = listener.local_addr()?;
    let router = ingress::http::router(ingress_tx, config.http.max_body_bytes);
    let mut shutdown = controller.subscribe();
    tasks.push(tokio::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.recv().await })
            .await;
        if let Err(err) = result {
            tracing::error!(%err, "HTTP ingress server error");
        }
    }));
    Ok(addr)
}

/// Bind and serve the operations endpoints, returning the bound address.
async fn spawn_ops_server(
    config: &Config,
    health: Health,
    metrics: metrics_exporter_prometheus::PrometheusHandle,
    peers: PeerRegistry,
    controller: &ShutdownController,
    tasks: &mut Vec<JoinHandle<()>>,
) -> anyhow::Result<SocketAddr> {
    let listener = TcpListener::bind(config.metrics.listen)
        .await
        .with_context(|| format!("failed to bind ops server on {}", config.metrics.listen))?;
    let addr = listener.local_addr()?;
    let router = observability::server::router_with_peers(health, metrics, peers);
    let mut shutdown = controller.subscribe();
    tasks.push(tokio::spawn(async move {
        let result = axum::serve(listener, router)
            .with_graceful_shutdown(async move { shutdown.recv().await })
            .await;
        if let Err(err) = result {
            tracing::error!(%err, "operations server error");
        }
    }));
    Ok(addr)
}
