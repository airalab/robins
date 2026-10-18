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
//! [`LightNode`]: the embedded smoldot light client behind a local JSON-RPC endpoint.

use super::{
    chain::Chains,
    config::{rpc_url, LightNodeConfig},
    error::{LightError, Result},
    server::{self, Limits},
    status::LightStatus,
};
use futures_util::FutureExt;
use smoldot_light::lifecycle_service::{LifecycleState, Phase};
use std::{net::SocketAddr, panic::AssertUnwindSafe, sync::Arc};
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};

/// A Robonomics light client serving a local WebSocket JSON-RPC endpoint.
///
/// The node runs smoldot for the Polkadot relay chain and the Robonomics
/// parachain and exposes the parachain on [`rpc_url`](Self::rpc_url). Any
/// Substrate JSON-RPC client can use the endpoint, in particular [`crate::Cps`]:
///
/// ```no_run
/// use libcps::{light::{LightNode, LightNodeConfig}, Cps};
///
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// let light = LightNode::start(LightNodeConfig::default().listen("127.0.0.1:0")).await?;
/// light.wait_ready().await?;
///
/// let cps = Cps::connect(light.rpc_url()).await?;
/// // ... use `cps` ...
///
/// light.shutdown().await?;
/// # Ok(())
/// # }
/// ```
///
/// Every connected client gets its own JSON-RPC session, so request IDs and
/// subscriptions of different clients are isolated. The node keeps no state
/// on disk: each start synchronizes from the chain specification checkpoints.
///
/// The node owns all its background tasks. [`shutdown`](Self::shutdown) stops
/// them and waits for completion; dropping the node asks them to stop without
/// waiting.
pub struct LightNode {
    addr: SocketAddr,
    url: String,
    status: Arc<watch::Sender<LightStatus>>,
    stop: watch::Sender<bool>,
    tasks: Vec<JoinHandle<()>>,
}

impl LightNode {
    /// Start the light client and its RPC endpoint.
    ///
    /// Returns once the endpoint is listening ([`LightStatus::RpcReady`]); use
    /// [`wait_ready`](Self::wait_ready) to wait for synchronization.
    ///
    /// # Errors
    ///
    /// [`LightError::Config`] for an invalid configuration,
    /// [`LightError::Bind`] if the listen address is unavailable,
    /// [`LightError::InvalidChainSpec`] / [`LightError::Bootstrap`] if the chains
    /// cannot be started.
    pub async fn start(config: LightNodeConfig) -> Result<Self> {
        let listen = config.listen_addr()?;
        let listener = TcpListener::bind(listen)
            .await
            .map_err(|source| LightError::Bind {
                addr: listen.to_string(),
                source,
            })?;
        let addr = listener.local_addr().map_err(|source| LightError::Bind {
            addr: listen.to_string(),
            source,
        })?;
        tracing::info!(%addr, "starting light node");

        // Chain setup parses the chain specifications, keep it off the reactor.
        let chains = {
            let config = config.clone();
            tokio::task::spawn_blocking(move || Chains::start(&config))
                .await
                .map_err(|e| LightError::Bootstrap(Box::new(e)))??
        };

        let (status, _) = watch::channel(LightStatus::Starting);
        let status = Arc::new(status);
        let (stop, stop_rx) = watch::channel(false);

        let monitor = tokio::spawn(monitor(chains.clone(), status.clone(), stop_rx.clone()));
        let limits = Limits {
            max_connections: config.max_connections,
            max_message_size: config.max_message_size,
        };
        let server = tokio::spawn({
            let status = status.clone();
            async move {
                let served = AssertUnwindSafe(server::serve(listener, chains, limits, stop_rx))
                    .catch_unwind()
                    .await;
                if served.is_err() {
                    fail(&status, "RPC server panicked");
                }
            }
        });

        status.send_if_modified(|current| {
            let starting = *current == LightStatus::Starting;
            if starting {
                *current = LightStatus::RpcReady;
            }
            starting
        });
        let url = rpc_url(addr);
        tracing::info!(%addr, %url, "light node RPC endpoint is listening");

        Ok(Self {
            addr,
            url,
            status,
            stop,
            tasks: vec![monitor, server],
        })
    }

    /// Address the RPC endpoint is bound to; useful with port `0`.
    pub fn rpc_addr(&self) -> SocketAddr {
        self.addr
    }

    /// WebSocket URL of the RPC endpoint, e.g. `ws://127.0.0.1:41233`.
    pub fn rpc_url(&self) -> &str {
        &self.url
    }

    /// Current status.
    pub fn current_status(&self) -> LightStatus {
        self.status.borrow().clone()
    }

    /// Watch status changes.
    pub fn status(&self) -> watch::Receiver<LightStatus> {
        self.status.subscribe()
    }

    /// Wait until the node is synchronized enough to answer finalized-state queries.
    ///
    /// # Errors
    ///
    /// [`LightError::Server`] if the node stops or fails before becoming ready.
    pub async fn wait_ready(&self) -> Result<()> {
        let mut status = self.status.subscribe();
        let reached = status
            .wait_for(|s| s.is_ready() || s.is_terminal())
            .await
            .map_err(|e| LightError::Server(Box::new(e)))?;
        match &*reached {
            LightStatus::Ready => Ok(()),
            other => Err(LightError::Server(other.to_string().into())),
        }
    }

    /// Wait until the node stops.
    ///
    /// Returns `Ok` after [`shutdown`](Self::shutdown) and an error if the node failed.
    pub async fn wait(&self) -> Result<()> {
        let mut status = self.status.subscribe();
        let reached = status
            .wait_for(LightStatus::is_terminal)
            .await
            .map_err(|e| LightError::Server(Box::new(e)))?;
        match &*reached {
            LightStatus::Failed(reason) => Err(LightError::Server(reason.clone().into())),
            _ => Ok(()),
        }
    }

    /// Stop the RPC server and the light client and wait for the tasks to finish.
    ///
    /// Open client connections are closed. The listen port is free afterwards.
    ///
    /// # Errors
    ///
    /// [`LightError::Shutdown`] if a background task panicked.
    pub async fn shutdown(mut self) -> Result<()> {
        tracing::info!("shutting down light node");
        let _ = self.stop.send(true);
        let mut failure = None;
        for task in std::mem::take(&mut self.tasks) {
            if let Err(error) = task.await {
                failure.get_or_insert(error);
            }
        }
        self.status.send_if_modified(|current| {
            let running = !current.is_terminal();
            if running {
                *current = LightStatus::Stopped;
            }
            running
        });
        tracing::info!("light node stopped");
        match failure {
            Some(error) => Err(LightError::Shutdown(Box::new(error))),
            None => Ok(()),
        }
    }
}

impl Drop for LightNode {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

/// Mark the node as failed, unless it already is terminal.
fn fail(status: &watch::Sender<LightStatus>, reason: &str) {
    tracing::error!(reason, "light node failed");
    status.send_if_modified(|current| {
        let running = !current.is_terminal();
        if running {
            *current = LightStatus::Failed(reason.to_string());
        }
        running
    });
}

/// Translate smoldot's lifecycle state into [`LightStatus`].
fn map_state(state: LifecycleState) -> LightStatus {
    match state.phase {
        Phase::Connecting => LightStatus::RpcReady,
        Phase::Syncing { at, target } => LightStatus::Syncing {
            finalized: at,
            target,
            peers: state.num_peers,
        },
        Phase::Ready => LightStatus::Ready,
    }
}

/// Follow the parachain synchronization state until shutdown.
async fn monitor(
    chains: Arc<Chains>,
    status: Arc<watch::Sender<LightStatus>>,
    mut stop: watch::Receiver<bool>,
) {
    let mut lifecycle = chains.lifecycle();
    loop {
        let state = tokio::select! {
            _ = stop.changed() => break,
            state = lifecycle.next() => state,
        };
        let Some(state) = state else { break };
        let next = map_state(state);
        let changed = status.send_if_modified(|current| {
            // `Starting` is replaced by `start` once the RPC endpoint listens.
            let update =
                *current != LightStatus::Starting && !current.is_terminal() && *current != next;
            if update {
                *current = next.clone();
            }
            update
        });
        if changed {
            tracing::info!(status = %next, "light node status changed");
        }
    }
}
