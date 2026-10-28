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
//! Shared connection to the Robonomics chain.
//!
//! Both CPS anchoring and CPS sensor authorization read the chain through
//! `libcps`. Without an explicit RPC endpoint each would start its own embedded
//! smoldot light client, which is expensive on an edge device. The
//! [`ChainRegistry`] hands out one [`Chain`] per endpoint (`None` being the
//! embedded light client), so every component that asks for the same endpoint
//! shares a single connection. Different explicitly configured endpoints are
//! different connections; to share one, configure the same endpoint (or the
//! common `[chain] endpoint`) for both subsystems.
//!
//! # Lifecycle
//!
//! The registry owns the whole life of a connection. The first request for an
//! endpoint starts exactly one supervisor task for it (never one per caller)
//! which
//!
//! 1. connects and, for the embedded light client, waits until it is
//!    synchronized;
//! 2. publishes the ready [`Chain`] to every waiting and future caller;
//! 3. retries transient failures with back-off, and reconnects when an embedded
//!    light client stops working.
//!
//! Initialization therefore never depends on a caller staying around: a request
//! that times out or is cancelled merely stops waiting, and neither cancels nor
//! restarts the shared initialization. Callers pick how to wait:
//!
//! - [`ChainRegistry::get`] waits for the connection to settle (used by the
//!   anchor, which has its own retry loop);
//! - [`ChainRegistry::try_get`] never waits and reports a connection that is
//!   still synchronizing as unavailable (used by authorization, which must fail
//!   closed instead of queueing behind a synchronization that may take minutes).
//!   An embedded light client can fall from ready back to synchronizing without
//!   stopping; it is reported as unavailable for that period and usable again
//!   once it has caught up, without being reconnected.
//!
//! Nothing here runs at daemon startup unless asked ([`ChainRegistry::prefetch`]),
//! so an unreachable chain cannot block ingestion.

use async_trait::async_trait;
use libcps::light::{LightError, LightNode, LightNodeConfig};
use libcps::{Cps, Error as CpsError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;

/// First delay before a failed connection attempt is repeated.
const RETRY_MIN: Duration = Duration::from_secs(1);

/// Upper bound of the delay between connection attempts.
const RETRY_MAX: Duration = Duration::from_secs(30);

/// Failure to establish a chain connection.
#[derive(Clone, Debug, thiserror::Error)]
pub enum ChainError {
    /// The chain is temporarily unreachable or still synchronizing; trying
    /// again may succeed.
    #[error("transient chain error: {0}")]
    Transient(String),
    /// The configuration is wrong; retrying cannot fix it.
    #[error("permanent chain error: {0}")]
    Permanent(String),
}

/// A connected `libcps` client, together with the embedded light node serving it.
pub struct Chain {
    cps: Cps,
    /// Embedded light node serving `cps`; declared after it so the client is
    /// dropped first.
    light: Option<LightNode>,
}

impl Chain {
    /// Connect to `endpoint`, or start an embedded [`LightNode`] on a free
    /// loopback port when `endpoint` is `None`. The light node lives as long as
    /// the returned [`Chain`].
    ///
    /// For the embedded node this returns only once it is synchronized enough
    /// to answer finalized-state queries, which can take a while. Callers that
    /// must not wait go through the [`ChainRegistry`], which runs this in its
    /// own task.
    pub async fn connect(endpoint: Option<&str>) -> Result<Self, ChainError> {
        let (light, url) = match endpoint {
            Some(url) => (None, url.to_string()),
            None => {
                let light = LightNode::start(LightNodeConfig::default())
                    .await
                    .map_err(classify_light)?;
                light.wait_ready().await.map_err(classify_light)?;
                let url = light.rpc_url().to_string();
                (Some(light), url)
            }
        };
        let cps = Cps::connect(&url).await.map_err(classify)?;
        Ok(Self { cps, light })
    }

    /// The `libcps` client.
    pub fn cps(&self) -> &Cps {
        &self.cps
    }
}

/// A live connection the registry can supervise.
#[async_trait]
pub trait Link: Send + Sync + 'static {
    /// Resolves once the connection is no longer usable and must be replaced.
    async fn lost(&self);

    /// Whether the connection currently follows the chain closely enough to
    /// answer finalized-state queries.
    ///
    /// An embedded light client that falls behind goes back to synchronizing
    /// without stopping, so [`lost`](Self::lost) does not fire; this reports
    /// that dip. Connections that cannot fall behind are always in sync.
    fn in_sync(&self) -> bool {
        true
    }
}

#[async_trait]
impl Link for Chain {
    async fn lost(&self) {
        match &self.light {
            Some(light) => {
                let _ = light.wait().await;
            }
            // An external endpoint is reconnected by the RPC client itself.
            None => std::future::pending().await,
        }
    }

    fn in_sync(&self) -> bool {
        self.light
            .as_ref()
            .is_none_or(|light| light.current_status().is_ready())
    }
}

/// Establishes a connection to an endpoint (`None` is the embedded light client).
#[async_trait]
pub trait Connect<C>: Send + Sync + 'static {
    /// Connect and wait until the connection is ready to serve queries.
    async fn connect(&self, endpoint: Option<&str>) -> Result<C, ChainError>;
}

/// The production [`Connect`]: `libcps` over a remote endpoint or a light node.
struct LibcpsConnector;

#[async_trait]
impl Connect<Chain> for LibcpsConnector {
    async fn connect(&self, endpoint: Option<&str>) -> Result<Chain, ChainError> {
        Chain::connect(endpoint).await
    }
}

/// Where the shared connection of one endpoint currently is.
enum State<C> {
    /// An attempt is in progress (connecting or synchronizing).
    Connecting,
    /// The last attempt failed transiently; the next one is scheduled.
    Retrying(String),
    /// Connected and ready.
    Ready(Arc<C>),
    /// Failed for good; nothing will be retried.
    Failed(String),
}

impl<C> State<C> {
    /// The connection, or why there is none. `Connecting` is reported as a
    /// transient error.
    fn resolve(&self) -> Result<Arc<C>, ChainError> {
        match self {
            State::Ready(chain) => Ok(chain.clone()),
            State::Connecting => Err(ChainError::Transient(
                "chain is still connecting or synchronizing".to_string(),
            )),
            State::Retrying(message) => Err(ChainError::Transient(message.clone())),
            State::Failed(message) => Err(ChainError::Permanent(message.clone())),
        }
    }
}

/// The supervised connection of one endpoint.
struct Slot<C> {
    state: Arc<watch::Sender<State<C>>>,
    supervisor: JoinHandle<()>,
}

impl<C> Drop for Slot<C> {
    fn drop(&mut self) {
        self.supervisor.abort();
    }
}

/// Supervised [`Chain`] connections, one per RPC endpoint.
///
/// See the [module documentation](self) for the lifecycle. The type parameter
/// only exists so tests can drive the lifecycle without a chain.
pub struct ChainRegistry<C = Chain> {
    connector: Arc<dyn Connect<C>>,
    retry: (Duration, Duration),
    slots: Mutex<HashMap<Option<String>, Arc<Slot<C>>>>,
}

impl Default for ChainRegistry<Chain> {
    fn default() -> Self {
        Self::new()
    }
}

impl ChainRegistry<Chain> {
    /// An empty registry connecting through `libcps`.
    pub fn new() -> Self {
        Self::with_connector(Arc::new(LibcpsConnector))
    }
}

impl<C: Link> ChainRegistry<C> {
    /// An empty registry connecting through `connector`.
    pub fn with_connector(connector: Arc<dyn Connect<C>>) -> Self {
        Self {
            connector,
            retry: (RETRY_MIN, RETRY_MAX),
            slots: Mutex::new(HashMap::new()),
        }
    }

    /// Override the delay between connection attempts.
    #[cfg(test)]
    pub(crate) fn with_retry(mut self, min: Duration, max: Duration) -> Self {
        self.retry = (min, max);
        self
    }

    /// Start connecting to `endpoint` in the background if nobody did yet.
    ///
    /// Returns immediately. Use it to begin synchronizing early without
    /// delaying startup.
    pub fn prefetch(&self, endpoint: Option<&str>) {
        self.slot(endpoint);
    }

    /// The shared connection for `endpoint`, waiting until it is ready.
    ///
    /// Cancelling or timing out this call only stops waiting: the shared
    /// connection attempt carries on. Returns the last error of the supervisor
    /// when it is between attempts, so callers can apply their own back-off.
    pub async fn get(&self, endpoint: Option<&str>) -> Result<Arc<C>, ChainError> {
        let slot = self.slot(endpoint);
        let mut state = slot.state.subscribe();
        let settled = state
            .wait_for(|state| !matches!(state, State::Connecting))
            .await
            .map_err(|_| ChainError::Permanent("chain supervisor stopped".to_string()))?;
        settled.resolve()
    }

    /// The shared connection for `endpoint` if it is ready right now.
    ///
    /// Never waits. A connection that is still synchronizing, or that fell
    /// back to synchronizing after having been ready, is reported as a
    /// transient error, so the caller can fail closed.
    pub fn try_get(&self, endpoint: Option<&str>) -> Result<Arc<C>, ChainError> {
        let chain = self.slot(endpoint).state.borrow().resolve()?;
        if chain.in_sync() {
            Ok(chain)
        } else {
            Err(ChainError::Transient(
                "chain is synchronizing again".to_string(),
            ))
        }
    }

    /// The slot of `endpoint`, starting its supervisor on first use.
    fn slot(&self, endpoint: Option<&str>) -> Arc<Slot<C>> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots
            .entry(endpoint.map(str::to_owned))
            .or_insert_with(|| {
                let (state, _) = watch::channel(State::Connecting);
                let state = Arc::new(state);
                let supervisor = tokio::spawn(supervise(
                    self.connector.clone(),
                    endpoint.map(str::to_owned),
                    state.clone(),
                    self.retry,
                ));
                Arc::new(Slot { state, supervisor })
            })
            .clone()
    }
}

/// Keep one endpoint connected: connect, publish, and reconnect on loss.
async fn supervise<C: Link>(
    connector: Arc<dyn Connect<C>>,
    endpoint: Option<String>,
    state: Arc<watch::Sender<State<C>>>,
    (retry_min, retry_max): (Duration, Duration),
) {
    let label = endpoint.as_deref().unwrap_or("embedded light client");
    let mut failures = 0u32;
    loop {
        state.send_replace(State::Connecting);
        match connector.connect(endpoint.as_deref()).await {
            Ok(chain) => {
                failures = 0;
                let chain = Arc::new(chain);
                tracing::info!(endpoint = label, "chain connection ready");
                state.send_replace(State::Ready(chain.clone()));
                chain.lost().await;
                tracing::warn!(endpoint = label, "chain connection lost; reconnecting");
            }
            Err(ChainError::Permanent(message)) => {
                tracing::error!(endpoint = label, %message, "chain connection failed permanently");
                state.send_replace(State::Failed(message));
                return;
            }
            Err(ChainError::Transient(message)) => {
                let delay = retry_min
                    .saturating_mul(2u32.saturating_pow(failures))
                    .min(retry_max);
                failures = failures.saturating_add(1);
                tracing::warn!(endpoint = label, %message, ?delay, "chain connection failed; retrying");
                state.send_replace(State::Retrying(message));
                tokio::time::sleep(delay).await;
            }
        }
    }
}

/// Light node failures: configuration problems are permanent, the rest may pass.
fn classify_light(error: LightError) -> ChainError {
    match error {
        LightError::Config(_) | LightError::InvalidChainSpec(_) => {
            ChainError::Permanent(error.to_string())
        }
        other => ChainError::Transient(other.to_string()),
    }
}

/// Connection failures are transient, anything else cannot be fixed by retrying.
fn classify(error: CpsError) -> ChainError {
    match error {
        CpsError::Connection(_) | CpsError::Rpc(_) | CpsError::Runtime(_) => {
            ChainError::Transient(error.to_string())
        }
        other => ChainError::Permanent(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use tokio::sync::Notify;

    /// A fake connection that can be told to drop.
    struct Fake {
        id: usize,
        lost: Notify,
        synced: AtomicBool,
    }

    #[async_trait]
    impl Link for Fake {
        async fn lost(&self) {
            self.lost.notified().await;
        }

        fn in_sync(&self) -> bool {
            self.synced.load(Ordering::SeqCst)
        }
    }

    /// Counts attempts and holds them at an optional gate until opened.
    #[derive(Default)]
    struct Connector {
        started: AtomicUsize,
        finished: AtomicUsize,
        /// Transient failures to hand out before succeeding.
        transient_failures: AtomicUsize,
        permanent: Mutex<bool>,
        gate: Option<Arc<Notify>>,
    }

    #[async_trait]
    impl Connect<Fake> for Arc<Connector> {
        async fn connect(&self, _endpoint: Option<&str>) -> Result<Fake, ChainError> {
            let id = self.started.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                gate.notified().await;
            }
            if *self.permanent.lock().unwrap() {
                return Err(ChainError::Permanent("wrong chain".into()));
            }
            if self
                .transient_failures
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
                .is_ok()
            {
                return Err(ChainError::Transient("rpc down".into()));
            }
            self.finished.fetch_add(1, Ordering::SeqCst);
            Ok(Fake {
                id,
                lost: Notify::new(),
                synced: AtomicBool::new(true),
            })
        }
    }

    fn registry(connector: &Arc<Connector>) -> ChainRegistry<Fake> {
        ChainRegistry::with_connector(Arc::new(connector.clone()))
            .with_retry(Duration::from_millis(2), Duration::from_millis(8))
    }

    fn gated() -> (Arc<Connector>, Arc<Notify>) {
        let gate = Arc::new(Notify::new());
        let connector = Arc::new(Connector {
            gate: Some(gate.clone()),
            ..Connector::default()
        });
        (connector, gate)
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_connection() {
        let connector = Arc::new(Connector::default());
        let chains = registry(&connector);

        let (a, b, c) = tokio::join!(chains.get(None), chains.get(None), chains.get(None));
        let (a, b, c) = (a.unwrap(), b.unwrap(), c.unwrap());
        assert!(Arc::ptr_eq(&a, &b) && Arc::ptr_eq(&b, &c));
        assert_eq!(connector.started.load(Ordering::SeqCst), 1);
        assert!(Arc::ptr_eq(&a, &chains.try_get(None).unwrap()));
    }

    #[tokio::test]
    async fn same_endpoint_is_shared_and_different_ones_are_not() {
        let connector = Arc::new(Connector::default());
        let chains = registry(&connector);

        let auth = chains.get(Some("ws://a")).await.unwrap();
        let anchor = chains.get(Some("ws://a")).await.unwrap();
        let other = chains.get(Some("ws://b")).await.unwrap();
        let embedded = chains.get(None).await.unwrap();

        assert!(Arc::ptr_eq(&auth, &anchor));
        assert!(!Arc::ptr_eq(&auth, &other));
        assert!(!Arc::ptr_eq(&auth, &embedded));
        assert_eq!(connector.started.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn a_waiter_timing_out_does_not_cancel_or_restart_initialization() {
        let (connector, gate) = gated();
        let chains = registry(&connector);

        for _ in 0..2 {
            let waited = tokio::time::timeout(Duration::from_millis(30), chains.get(None)).await;
            assert!(waited.is_err(), "still synchronizing");
            assert_eq!(connector.started.load(Ordering::SeqCst), 1);
        }

        gate.notify_one();
        let chain = chains.get(None).await.unwrap();
        assert_eq!(chain.id, 0);
        assert_eq!(connector.started.load(Ordering::SeqCst), 1);
        assert_eq!(connector.finished.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn try_get_fails_fast_while_synchronizing() {
        let (connector, gate) = gated();
        let chains = registry(&connector);

        for _ in 0..3 {
            assert!(matches!(
                chains.try_get(None),
                Err(ChainError::Transient(_))
            ));
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(connector.started.load(Ordering::SeqCst), 1);

        gate.notify_one();
        chains.get(None).await.unwrap();
        assert!(chains.try_get(None).is_ok());
        assert_eq!(connector.started.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn try_get_gates_on_a_connection_that_fell_behind() {
        let connector = Arc::new(Connector::default());
        let chains = registry(&connector);
        let chain = chains.get(None).await.unwrap();
        assert!(chains.try_get(None).is_ok(), "ready");

        chain.synced.store(false, Ordering::SeqCst);
        assert!(matches!(
            chains.try_get(None),
            Err(ChainError::Transient(_))
        ));
        assert_eq!(connector.started.load(Ordering::SeqCst), 1, "not restarted");

        chain.synced.store(true, Ordering::SeqCst);
        assert!(Arc::ptr_eq(&chain, &chains.try_get(None).unwrap()));
        assert_eq!(connector.started.load(Ordering::SeqCst), 1, "not restarted");
    }

    #[tokio::test]
    async fn transient_failures_are_retried_in_the_background() {
        let connector = Arc::new(Connector::default());
        connector.transient_failures.store(2, Ordering::SeqCst);
        let chains = registry(&connector);

        // The first caller sees the failure, the supervisor keeps trying.
        assert!(matches!(
            chains.get(None).await,
            Err(ChainError::Transient(_))
        ));
        let chain = loop {
            if let Ok(chain) = chains.try_get(None) {
                break chain;
            }
            tokio::time::sleep(Duration::from_millis(2)).await;
        };
        assert_eq!(chain.id, 2);
        assert_eq!(connector.started.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn permanent_failures_are_final() {
        let connector = Arc::new(Connector::default());
        *connector.permanent.lock().unwrap() = true;
        let chains = registry(&connector);

        assert!(matches!(
            chains.get(None).await,
            Err(ChainError::Permanent(_))
        ));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(matches!(
            chains.try_get(None),
            Err(ChainError::Permanent(_))
        ));
        assert_eq!(connector.started.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_lost_connection_is_replaced() {
        let connector = Arc::new(Connector::default());
        let chains = registry(&connector);

        let first = chains.get(None).await.unwrap();
        assert_eq!(first.id, 0);
        first.lost.notify_one();

        let second = loop {
            match chains.try_get(None) {
                Ok(chain) if chain.id == 1 => break chain,
                _ => tokio::time::sleep(Duration::from_millis(2)).await,
            }
        };
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(connector.started.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn prefetch_starts_connecting_without_waiting() {
        let (connector, gate) = gated();
        let chains = registry(&connector);

        chains.prefetch(Some("ws://a"));
        chains.prefetch(Some("ws://a"));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(connector.started.load(Ordering::SeqCst), 1);

        gate.notify_one();
        assert!(chains.get(Some("ws://a")).await.is_ok());
        assert_eq!(connector.started.load(Ordering::SeqCst), 1);
    }
}
