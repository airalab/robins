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
//! The `cps` authorization policy.
//!
//! A verified sensor is authorized when its public key is bound to a CPS node
//! that is a **strict descendant** of the configured `sensors.social` root node,
//! at any depth. The root itself is never a sensor.
//!
//! ```text
//!   SensorId ─▶ cache ─hit─▶ decision
//!                 │miss
//!                 ▼
//!   finalized snapshot ─▶ address(account).node_id() ─▶ parent() … ─▶ root?
//! ```
//!
//! # Caching
//!
//! Decisions live in one bounded, process-local LRU shared by allow and deny
//! entries (the capacity is a hard bound on both together). Nothing is
//! persisted.
//!
//! | Decision                                    | Retention                       |
//! | ------------------------------------------- | ------------------------------- |
//! | allowed                                     | until eviction or restart       |
//! | denied                                      | `negative_ttl`                  |
//! | RPC error, timeout, wrong chain, bad tree   | never cached                    |
//!
//! A cached allow therefore keeps working during a chain outage, while a sensor
//! that is not cached fails closed. The trade-off is that a CPS node that is
//! moved out of the subtree stays authorized until its entry is evicted or the
//! gateway restarts; CPS nodes are normally not removed.

use super::{AuthError, AuthPolicy};
use crate::chain::ChainRegistry;
use crate::config::{AuthConfig, AuthMode};
use crate::observability::metrics::{
    AUTH_CPS_CACHE_ENTRIES, AUTH_CPS_CACHE_HIT_TOTAL, AUTH_CPS_CACHE_MISS_TOTAL,
};
use crate::protocol::SensorId;
use async_trait::async_trait;
use libcps::{AccountId, NodeId};
use lru::LruCache;
use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::OnceCell;

/// Most ancestors followed before a lookup is declared inconsistent.
///
/// This is a safety net against corrupted trees, not a limit on where sensors
/// may live.
const MAX_ANCESTOR_DEPTH: usize = 256;

/// Position of a CPS node in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lineage {
    /// The node does not exist.
    Missing,
    /// The node exists and has no parent.
    Root,
    /// The node exists and hangs below this parent.
    Parent(NodeId),
}

/// Read access to the CPS chain, kept narrow so tests can run without one.
#[async_trait]
pub trait CpsReader: Send + Sync {
    /// Check that the reader talks to the intended chain and that the
    /// configured root exists. Called until it succeeds once.
    async fn verify(&self) -> Result<(), AuthError>;

    /// Pin a consistent view of the chain for one authorization decision.
    async fn snapshot(&self) -> Result<Box<dyn CpsSnapshot>, AuthError>;
}

/// A consistent view of the CPS tree at a single finalized block.
#[async_trait]
pub trait CpsSnapshot: Send + Sync {
    /// The node bound to the identity of `sensor_id`, `None` if unbound.
    async fn node_of(&self, sensor_id: &SensorId) -> Result<Option<NodeId>, AuthError>;

    /// Where `node` sits in the tree.
    async fn lineage(&self, node: NodeId) -> Result<Lineage, AuthError>;
}

/// A cached decision and when it stops being valid.
#[derive(Clone, Copy, Debug)]
struct Entry {
    allowed: bool,
    /// `None` keeps the entry until eviction.
    expires: Option<Instant>,
}

/// Time source, replaceable so tests can step over TTLs.
type Clock = Box<dyn Fn() -> Instant + Send + Sync>;

/// Authorize sensors that live below a CPS root node.
pub struct CpsAuth {
    reader: Arc<dyn CpsReader>,
    root: NodeId,
    negative_ttl: Duration,
    query_timeout: Duration,
    /// Set once the reader confirmed chain identity and the root node.
    verified: OnceCell<()>,
    /// Never held across an `.await`.
    cache: Mutex<LruCache<[u8; 32], Entry>>,
    clock: Clock,
}

impl CpsAuth {
    /// Create a policy over `reader` that authorizes descendants of `root`.
    ///
    /// `capacity` bounds allow and deny entries together.
    pub fn new(
        reader: Arc<dyn CpsReader>,
        root: NodeId,
        capacity: NonZeroUsize,
        negative_ttl: Duration,
        query_timeout: Duration,
    ) -> Self {
        Self {
            reader,
            root,
            negative_ttl,
            query_timeout,
            verified: OnceCell::new(),
            cache: Mutex::new(LruCache::new(capacity)),
            clock: Box::new(Instant::now),
        }
    }

    /// Build the policy from `[auth]`, reading the chain at `endpoint` (`None`:
    /// the embedded light client) through `chains`.
    ///
    /// Nothing is connected here: the chain is reached on the first lookup (or
    /// by [`ChainRegistry::prefetch`]), so an unreachable chain never blocks
    /// startup and never opens the gateway.
    ///
    /// # Errors
    ///
    /// A description of the offending `[auth]` setting.
    pub fn from_config(
        config: &AuthConfig,
        endpoint: Option<&str>,
        chains: Arc<ChainRegistry>,
    ) -> Result<Self, String> {
        let root = NodeId(
            config
                .cps_root_node_id
                .ok_or("auth.mode = \"cps\" requires auth.cps_root_node_id")?,
        );
        let expected_genesis = config.expected_genesis_hash()?;
        let capacity = NonZeroUsize::new(config.cps_cache_capacity)
            .ok_or("auth.cps_cache_capacity must be greater than zero")?;
        let reader = LibcpsReader {
            chains,
            endpoint: endpoint.map(str::to_owned),
            expected_genesis,
            root,
        };
        Ok(Self::new(
            Arc::new(reader),
            root,
            capacity,
            Duration::from_secs(config.cps_negative_ttl_secs),
            Duration::from_secs(config.cps_query_timeout_secs),
        ))
    }

    /// Replace the time source.
    #[cfg(test)]
    fn with_clock(mut self, clock: Clock) -> Self {
        self.clock = clock;
        self
    }

    /// A live cached decision for `key`, dropping it if it has expired.
    fn cached(&self, key: &[u8; 32]) -> Option<bool> {
        let now = (self.clock)();
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let entry = *cache.get(key)?;
        if entry.expires.is_some_and(|expires| expires <= now) {
            cache.pop(key);
            metrics::gauge!(AUTH_CPS_CACHE_ENTRIES).set(cache.len() as f64);
            return None;
        }
        Some(entry.allowed)
    }

    /// Remember a decision; the least recently used entry makes room if full.
    fn store(&self, key: [u8; 32], allowed: bool) {
        let expires = (!allowed).then(|| (self.clock)() + self.negative_ttl);
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache.put(key, Entry { allowed, expires });
        metrics::gauge!(AUTH_CPS_CACHE_ENTRIES).set(cache.len() as f64);
    }

    /// Number of cached decisions.
    pub fn cached_entries(&self) -> usize {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Decide from the chain, bypassing the cache.
    async fn lookup(&self, sensor_id: &SensorId) -> Result<bool, AuthError> {
        self.verified
            .get_or_try_init(|| self.reader.verify())
            .await?;
        let snapshot = self.reader.snapshot().await?;

        let Some(mut current) = snapshot.node_of(sensor_id).await? else {
            return Ok(false);
        };
        if current == self.root {
            return Ok(false);
        }
        let mut visited = HashSet::from([current]);
        for _ in 0..MAX_ANCESTOR_DEPTH {
            match snapshot.lineage(current).await? {
                Lineage::Missing => {
                    return Err(AuthError::Inconsistent(format!(
                        "node {current} is missing from the tree"
                    )))
                }
                Lineage::Root => return Ok(false),
                Lineage::Parent(parent) if parent == self.root => return Ok(true),
                Lineage::Parent(parent) => {
                    if !visited.insert(parent) {
                        return Err(AuthError::Inconsistent(format!(
                            "ancestor cycle at node {parent}"
                        )));
                    }
                    current = parent;
                }
            }
        }
        Err(AuthError::Inconsistent(format!(
            "more than {MAX_ANCESTOR_DEPTH} ancestors"
        )))
    }
}

#[async_trait]
impl AuthPolicy for CpsAuth {
    fn mode(&self) -> AuthMode {
        AuthMode::Cps
    }

    fn try_authorize(&self, sensor_id: &SensorId) -> Option<bool> {
        let allowed = self.cached(sensor_id.as_bytes())?;
        let decision = if allowed { "allow" } else { "deny" };
        metrics::counter!(AUTH_CPS_CACHE_HIT_TOTAL, "decision" => decision).increment(1);
        Some(allowed)
    }

    async fn authorize(&self, sensor_id: &SensorId) -> Result<bool, AuthError> {
        // The pipeline normally got here after a miss, but another lookup may
        // have filled the cache in the meantime.
        if let Some(allowed) = self.try_authorize(sensor_id) {
            return Ok(allowed);
        }
        let key = *sensor_id.as_bytes();
        metrics::counter!(AUTH_CPS_CACHE_MISS_TOTAL).increment(1);

        let allowed = tokio::time::timeout(self.query_timeout, self.lookup(sensor_id))
            .await
            .map_err(|_| AuthError::Timeout)??;
        self.store(key, allowed);
        Ok(allowed)
    }
}

/// [`CpsReader`] backed by `libcps`.
struct LibcpsReader {
    chains: Arc<ChainRegistry>,
    endpoint: Option<String>,
    expected_genesis: [u8; 32],
    root: NodeId,
}

impl LibcpsReader {
    /// The shared connection if it is ready; never waits for it, so a chain
    /// that is still synchronizing fails the lookup (closed) at once instead of
    /// holding a lookup slot, and the shared initialization is never touched.
    fn chain(&self) -> Result<Arc<crate::chain::Chain>, AuthError> {
        self.chains
            .try_get(self.endpoint.as_deref())
            .map_err(|e| AuthError::Unavailable(e.to_string()))
    }
}

#[async_trait]
impl CpsReader for LibcpsReader {
    async fn verify(&self) -> Result<(), AuthError> {
        let chain = self.chain()?;
        let actual = chain.cps().genesis_hash().0;
        if actual != self.expected_genesis {
            return Err(AuthError::ChainMismatch {
                expected: hex::encode(self.expected_genesis),
                actual: hex::encode(actual),
            });
        }
        let at = chain.cps().finalized().await.map_err(lookup_error)?;
        match at.node(self.root).info().await.map_err(lookup_error)? {
            Some(_) => Ok(()),
            None => Err(AuthError::RootMissing(self.root)),
        }
    }

    async fn snapshot(&self) -> Result<Box<dyn CpsSnapshot>, AuthError> {
        let chain = self.chain()?;
        let at = chain.cps().finalized().await.map_err(lookup_error)?;
        Ok(Box::new(LibcpsSnapshot { at }))
    }
}

/// One finalized block of the chain.
struct LibcpsSnapshot {
    at: libcps::CpsAt,
}

#[async_trait]
impl CpsSnapshot for LibcpsSnapshot {
    async fn node_of(&self, sensor_id: &SensorId) -> Result<Option<NodeId>, AuthError> {
        let account = AccountId::from(*sensor_id.as_bytes());
        self.at
            .address(account)
            .node_id()
            .await
            .map_err(lookup_error)
    }

    async fn lineage(&self, node: NodeId) -> Result<Lineage, AuthError> {
        let info = self.at.node(node).info().await.map_err(lookup_error)?;
        Ok(match info {
            None => Lineage::Missing,
            Some(info) => info.parent.map_or(Lineage::Root, Lineage::Parent),
        })
    }
}

fn lookup_error(error: libcps::Error) -> AuthError {
    AuthError::Lookup(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::AuthLimits;
    use crate::protocol::SensorIdentity;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ROOT: NodeId = NodeId(1);

    fn sensor(seed: u8) -> SensorId {
        SensorIdentity::from_secret_bytes(&[seed; 32]).sensor_id()
    }

    /// An in-memory chain that counts snapshots and can be taken offline.
    #[derive(Default)]
    struct MockChain {
        bindings: Mutex<HashMap<[u8; 32], NodeId>>,
        tree: Mutex<HashMap<NodeId, Option<NodeId>>>,
        snapshots: AtomicUsize,
        verifies: AtomicUsize,
        offline: Mutex<Option<AuthError>>,
        verify_error: Mutex<Option<AuthError>>,
    }

    impl MockChain {
        /// A tree `ROOT(1) -> 2 -> 3`, plus an unrelated root `10 -> 11`.
        fn new() -> Arc<Self> {
            let chain = Arc::new(Self::default());
            {
                let mut tree = chain.tree.lock().unwrap();
                tree.insert(ROOT, None);
                tree.insert(NodeId(2), Some(ROOT));
                tree.insert(NodeId(3), Some(NodeId(2)));
                tree.insert(NodeId(10), None);
                tree.insert(NodeId(11), Some(NodeId(10)));
            }
            chain
        }

        fn bind(&self, sensor: &SensorId, node: u64) {
            self.bindings
                .lock()
                .unwrap()
                .insert(*sensor.as_bytes(), NodeId(node));
        }

        fn set_parent(&self, node: u64, parent: Option<u64>) {
            self.tree
                .lock()
                .unwrap()
                .insert(NodeId(node), parent.map(NodeId));
        }

        fn go_offline(&self) {
            *self.offline.lock().unwrap() = Some(AuthError::Lookup("rpc down".into()));
        }

        fn go_online(&self) {
            *self.offline.lock().unwrap() = None;
        }

        fn snapshots(&self) -> usize {
            self.snapshots.load(Ordering::SeqCst)
        }
    }

    struct MockReader(Arc<MockChain>);

    fn clone_error(error: &AuthError) -> AuthError {
        AuthError::Lookup(error.to_string())
    }

    #[async_trait]
    impl CpsReader for MockReader {
        async fn verify(&self) -> Result<(), AuthError> {
            self.0.verifies.fetch_add(1, Ordering::SeqCst);
            match &*self.0.verify_error.lock().unwrap() {
                Some(error) => Err(clone_error(error)),
                None => Ok(()),
            }
        }

        async fn snapshot(&self) -> Result<Box<dyn CpsSnapshot>, AuthError> {
            self.0.snapshots.fetch_add(1, Ordering::SeqCst);
            match &*self.0.offline.lock().unwrap() {
                Some(error) => Err(clone_error(error)),
                None => Ok(Box::new(MockSnapshot(self.0.clone()))),
            }
        }
    }

    struct MockSnapshot(Arc<MockChain>);

    #[async_trait]
    impl CpsSnapshot for MockSnapshot {
        async fn node_of(&self, sensor_id: &SensorId) -> Result<Option<NodeId>, AuthError> {
            Ok(self
                .0
                .bindings
                .lock()
                .unwrap()
                .get(sensor_id.as_bytes())
                .copied())
        }

        async fn lineage(&self, node: NodeId) -> Result<Lineage, AuthError> {
            Ok(match self.0.tree.lock().unwrap().get(&node) {
                None => Lineage::Missing,
                Some(None) => Lineage::Root,
                Some(Some(parent)) => Lineage::Parent(*parent),
            })
        }
    }

    fn policy(chain: &Arc<MockChain>, capacity: usize) -> CpsAuth {
        CpsAuth::new(
            Arc::new(MockReader(chain.clone())),
            ROOT,
            NonZeroUsize::new(capacity).unwrap(),
            Duration::from_secs(60),
            Duration::from_secs(1),
        )
    }

    /// A clock that only moves when the returned handle is advanced.
    fn manual_clock() -> (Clock, Arc<Mutex<Duration>>) {
        let start = Instant::now();
        let offset = Arc::new(Mutex::new(Duration::ZERO));
        let shared = offset.clone();
        (Box::new(move || start + *shared.lock().unwrap()), offset)
    }

    #[tokio::test]
    async fn descendant_at_any_depth_is_allowed() {
        let chain = MockChain::new();
        let (child, grandchild) = (sensor(1), sensor(2));
        chain.bind(&child, 2);
        chain.bind(&grandchild, 3);
        let policy = policy(&chain, 8);

        assert_eq!(policy.mode(), AuthMode::Cps);
        assert!(policy.authorize(&child).await.unwrap());
        assert!(policy.authorize(&grandchild).await.unwrap());
    }

    #[tokio::test]
    async fn unbound_outside_and_root_are_denied() {
        let chain = MockChain::new();
        let (outside, at_root) = (sensor(2), sensor(3));
        chain.bind(&outside, 11);
        chain.bind(&at_root, 1);
        let policy = policy(&chain, 8);

        assert!(!policy.authorize(&sensor(1)).await.unwrap());
        assert!(!policy.authorize(&outside).await.unwrap());
        assert!(!policy.authorize(&at_root).await.unwrap());
    }

    #[tokio::test]
    async fn corrupted_ancestry_is_an_error_and_not_cached() {
        let chain = MockChain::new();
        let (missing, cyclic) = (sensor(1), sensor(2));
        chain.bind(&missing, 99);
        chain.bind(&cyclic, 20);
        chain.set_parent(20, Some(21));
        chain.set_parent(21, Some(20));
        let policy = policy(&chain, 8);

        assert!(matches!(
            policy.authorize(&missing).await,
            Err(AuthError::Inconsistent(_))
        ));
        assert!(matches!(
            policy.authorize(&cyclic).await,
            Err(AuthError::Inconsistent(_))
        ));
        assert_eq!(policy.cached_entries(), 0);
    }

    #[tokio::test]
    async fn overly_deep_ancestry_is_an_error() {
        let chain = MockChain::new();
        for node in 100..100 + MAX_ANCESTOR_DEPTH as u64 + 5 {
            chain.set_parent(node, Some(node + 1));
        }
        let deep = sensor(1);
        chain.bind(&deep, 100);
        let policy = policy(&chain, 8);

        assert!(matches!(
            policy.authorize(&deep).await,
            Err(AuthError::Inconsistent(_))
        ));
    }

    #[tokio::test]
    async fn positive_decision_is_cached_without_ttl() {
        let chain = MockChain::new();
        let allowed = sensor(1);
        chain.bind(&allowed, 3);
        let (clock, offset) = manual_clock();
        let policy = policy(&chain, 8).with_clock(clock);

        assert!(policy.authorize(&allowed).await.unwrap());
        assert_eq!(chain.snapshots(), 1);

        *offset.lock().unwrap() = Duration::from_secs(10 * 24 * 3600);
        assert!(policy.authorize(&allowed).await.unwrap());
        assert_eq!(chain.snapshots(), 1, "an allow must not be re-queried");
    }

    #[tokio::test]
    async fn negative_decision_expires() {
        let chain = MockChain::new();
        let late = sensor(1);
        let (clock, offset) = manual_clock();
        let policy = policy(&chain, 8).with_clock(clock);

        assert!(!policy.authorize(&late).await.unwrap());
        assert!(!policy.authorize(&late).await.unwrap());
        assert_eq!(chain.snapshots(), 1);

        chain.bind(&late, 2);
        *offset.lock().unwrap() = Duration::from_secs(59);
        assert!(!policy.authorize(&late).await.unwrap());

        *offset.lock().unwrap() = Duration::from_secs(61);
        assert!(policy.authorize(&late).await.unwrap());
        assert_eq!(chain.snapshots(), 2);
    }

    #[tokio::test]
    async fn cache_never_exceeds_capacity_and_evicted_entries_requery() {
        let chain = MockChain::new();
        let policy = policy(&chain, 4);
        for seed in 0..32u8 {
            let id = sensor(seed);
            chain.bind(&id, 2);
            assert!(policy.authorize(&id).await.unwrap());
            assert!(policy.cached_entries() <= 4);
        }
        assert_eq!(policy.cached_entries(), 4);
        assert_eq!(chain.snapshots(), 32);

        assert!(policy.authorize(&sensor(0)).await.unwrap());
        assert_eq!(chain.snapshots(), 33, "evicted sensor is looked up again");
        assert!(policy.authorize(&sensor(31)).await.unwrap());
        assert_eq!(chain.snapshots(), 33, "recent sensor is still cached");
    }

    #[tokio::test]
    async fn lookup_failures_fail_closed_and_are_not_cached() {
        let chain = MockChain::new();
        let id = sensor(1);
        chain.bind(&id, 2);
        let policy = policy(&chain, 8);

        chain.go_offline();
        assert!(policy.authorize(&id).await.is_err());
        assert_eq!(policy.cached_entries(), 0);

        chain.go_online();
        assert!(policy.authorize(&id).await.unwrap());
    }

    #[tokio::test]
    async fn cached_allow_survives_an_outage_but_a_miss_fails_closed() {
        let chain = MockChain::new();
        let (known, unknown) = (sensor(1), sensor(2));
        chain.bind(&known, 2);
        chain.bind(&unknown, 2);
        let policy = policy(&chain, 8);

        assert!(policy.authorize(&known).await.unwrap());
        chain.go_offline();
        assert!(policy.authorize(&known).await.unwrap());
        assert!(policy.authorize(&unknown).await.is_err());
    }

    #[tokio::test]
    async fn failed_verification_blocks_decisions_until_it_succeeds() {
        let chain = MockChain::new();
        let id = sensor(1);
        chain.bind(&id, 2);
        *chain.verify_error.lock().unwrap() = Some(AuthError::ChainMismatch {
            expected: "a".into(),
            actual: "b".into(),
        });
        let policy = policy(&chain, 8);

        assert!(policy.authorize(&id).await.is_err());
        assert_eq!(chain.snapshots(), 0, "no lookup on an unverified chain");
        assert_eq!(policy.cached_entries(), 0);

        *chain.verify_error.lock().unwrap() = None;
        assert!(policy.authorize(&id).await.unwrap());
        assert!(policy.authorize(&sensor(2)).await.is_ok());
        assert_eq!(chain.verifies.load(Ordering::SeqCst), 2, "verified once");
    }

    #[tokio::test]
    async fn slow_lookup_times_out() {
        struct Stuck;

        #[async_trait]
        impl CpsReader for Stuck {
            async fn verify(&self) -> Result<(), AuthError> {
                Ok(())
            }

            async fn snapshot(&self) -> Result<Box<dyn CpsSnapshot>, AuthError> {
                std::future::pending().await
            }
        }

        let policy = CpsAuth::new(
            Arc::new(Stuck),
            ROOT,
            NonZeroUsize::new(8).unwrap(),
            Duration::from_secs(60),
            Duration::from_millis(20),
        );
        assert!(matches!(
            policy.authorize(&sensor(1)).await,
            Err(AuthError::Timeout)
        ));
        assert_eq!(policy.cached_entries(), 0);
    }

    #[test]
    fn from_config_validates_settings() {
        let chains = Arc::new(ChainRegistry::new());
        let mut config = AuthConfig {
            mode: AuthMode::Cps,
            cps_root_node_id: Some(5),
            cps_expected_genesis_hash: Some(format!("0x{}", "11".repeat(32))),
            ..AuthConfig::default()
        };
        assert!(CpsAuth::from_config(&config, None, chains.clone()).is_ok());

        config.cps_cache_capacity = 0;
        assert!(CpsAuth::from_config(&config, None, chains.clone()).is_err());
        config.cps_cache_capacity = 1;
        config.cps_expected_genesis_hash = None;
        assert!(CpsAuth::from_config(&config, None, chains).is_err());
    }

    /// A reader whose snapshots wait for permits, to hold lookups open.
    struct SlowReader {
        inner: MockReader,
        gate: Arc<tokio::sync::Semaphore>,
        running: AtomicUsize,
        max_running: AtomicUsize,
    }

    #[async_trait]
    impl CpsReader for Arc<SlowReader> {
        async fn verify(&self) -> Result<(), AuthError> {
            self.inner.verify().await
        }

        async fn snapshot(&self) -> Result<Box<dyn CpsSnapshot>, AuthError> {
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_running.fetch_max(now, Ordering::SeqCst);
            self.gate.acquire().await.unwrap().forget();
            self.running.fetch_sub(1, Ordering::SeqCst);
            self.inner.snapshot().await
        }
    }

    /// A CPS policy over a chain whose lookups stay open until released.
    fn slow_policy(chain: &Arc<MockChain>) -> (Arc<CpsAuth>, Arc<SlowReader>) {
        let reader = Arc::new(SlowReader {
            inner: MockReader(chain.clone()),
            gate: Arc::new(tokio::sync::Semaphore::new(0)),
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
        });
        let policy = CpsAuth::new(
            Arc::new(reader.clone()),
            ROOT,
            NonZeroUsize::new(64).unwrap(),
            Duration::from_secs(60),
            Duration::from_secs(5),
        );
        (Arc::new(policy), reader)
    }

    /// Run `messages` through a pipeline over `policy`; returns the delivery
    /// receiver and the ingress sender.
    fn run_pipeline(
        policy: Arc<CpsAuth>,
        limits: AuthLimits,
    ) -> (
        crate::ingress::IngressSender,
        tokio::sync::mpsc::Receiver<Arc<crate::pipeline::AcceptedMessage>>,
        crate::shutdown::ShutdownController,
    ) {
        struct Shared(Arc<CpsAuth>);

        #[async_trait]
        impl AuthPolicy for Shared {
            fn mode(&self) -> AuthMode {
                AuthMode::Cps
            }

            fn try_authorize(&self, sensor_id: &SensorId) -> Option<bool> {
                self.0.try_authorize(sensor_id)
            }

            async fn authorize(&self, sensor_id: &SensorId) -> Result<bool, AuthError> {
                self.0.authorize(sensor_id).await
            }
        }

        let (sink, delivered) = crate::pipeline::ChannelSink::new("test", 256);
        let pipeline = crate::pipeline::Pipeline::new(
            Box::new(Shared(policy)),
            crate::pipeline::DedupCache::new(256, Duration::from_secs(60)),
            vec![Box::new(sink)],
        )
        .with_auth_limits(limits);
        let (tx, rx) = crate::ingress::channel(256);
        let controller = crate::shutdown::ShutdownController::new();
        tokio::spawn(pipeline.run(rx, controller.subscribe()));
        (tx, delivered, controller)
    }

    fn message(seed: u8, payload: &[u8]) -> crate::ingress::IngressMessage {
        use crate::protocol::{encode_envelope, sign_message, SignOptions};
        let identity = SensorIdentity::from_secret_bytes(&[seed; 32]);
        let envelope = sign_message(&identity, payload, SignOptions::default());
        crate::ingress::IngressMessage {
            raw_envelope: bytes::Bytes::from(encode_envelope(&envelope)),
            envelope,
            transport: crate::ingress::TransportMetadata::http(None),
            received_at: std::time::SystemTime::now(),
        }
    }

    async fn delivered_from(
        delivered: &mut tokio::sync::mpsc::Receiver<Arc<crate::pipeline::AcceptedMessage>>,
    ) -> SensorId {
        tokio::time::timeout(Duration::from_secs(5), delivered.recv())
            .await
            .expect("delivered in time")
            .expect("sink open")
            .sensor_id
            .clone()
    }

    #[tokio::test]
    async fn a_slow_chain_miss_does_not_delay_a_cached_sensor() {
        let chain = MockChain::new();
        let (cached, fresh) = (sensor(1), sensor(2));
        chain.bind(&cached, 2);
        chain.bind(&fresh, 3);
        let (policy, reader) = slow_policy(&chain);

        reader.gate.add_permits(1);
        assert!(policy.authorize(&cached).await.unwrap());
        assert_eq!(chain.snapshots(), 1);

        let (tx, mut delivered, _controller) = run_pipeline(policy, AuthLimits::default());
        tx.send(message(2, b"waits for the chain")).await.unwrap();
        tx.send(message(1, b"cached")).await.unwrap();

        assert_eq!(delivered_from(&mut delivered).await, cached);
        assert!(delivered.try_recv().is_err(), "the miss is still pending");

        reader.gate.add_permits(1);
        assert_eq!(delivered_from(&mut delivered).await, fresh);
    }

    #[tokio::test]
    async fn concurrent_misses_cause_a_single_chain_query() {
        let chain = MockChain::new();
        let id = sensor(1);
        chain.bind(&id, 2);
        let (policy, reader) = slow_policy(&chain);
        let (tx, mut delivered, _controller) = run_pipeline(policy.clone(), AuthLimits::default());

        for n in 0..20u8 {
            tx.send(message(1, &[n])).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        reader.gate.add_permits(100);

        for _ in 0..20 {
            assert_eq!(delivered_from(&mut delivered).await, id);
        }
        assert_eq!(chain.snapshots(), 1);
        assert_eq!(chain.verifies.load(Ordering::SeqCst), 1);
        assert_eq!(policy.cached_entries(), 1);
    }

    #[tokio::test]
    async fn chain_queries_stay_within_the_configured_concurrency() {
        let chain = MockChain::new();
        for seed in 0..40u8 {
            chain.bind(&sensor(seed), 2);
        }
        let (policy, reader) = slow_policy(&chain);
        let limits = AuthLimits {
            max_concurrent: 4,
            max_pending: 40,
        };
        let (tx, mut delivered, _controller) = run_pipeline(policy, limits);

        for seed in 0..40u8 {
            tx.send(message(seed, b"burst")).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(reader.running.load(Ordering::SeqCst), 4);

        reader.gate.add_permits(100);
        for _ in 0..40 {
            delivered_from(&mut delivered).await;
        }
        assert_eq!(reader.max_running.load(Ordering::SeqCst), 4);
        assert_eq!(chain.snapshots(), 40);
    }

    #[tokio::test]
    async fn an_unsynchronized_chain_fails_closed_without_caching_or_waiting() {
        // Nothing listens on this port: the shared connection never gets ready.
        let config = AuthConfig {
            mode: AuthMode::Cps,
            cps_root_node_id: Some(1),
            cps_expected_genesis_hash: Some(format!("0x{}", "00".repeat(32))),
            ..AuthConfig::default()
        };
        let policy = CpsAuth::from_config(
            &config,
            Some("ws://127.0.0.1:1"),
            Arc::new(ChainRegistry::new()),
        )
        .unwrap();

        for _ in 0..3 {
            let started = Instant::now();
            assert!(matches!(
                policy.authorize(&sensor(1)).await,
                Err(AuthError::Unavailable(_))
            ));
            assert!(started.elapsed() < Duration::from_secs(1), "must not wait");
        }
        assert_eq!(policy.cached_entries(), 0, "no denial is remembered");
    }
}
