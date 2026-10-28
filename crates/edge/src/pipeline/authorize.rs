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
//! The bounded asynchronous authorization stage.
//!
//! The pipeline handles messages in arrival order, but an [`AuthPolicy`] may
//! need a slow chain lookup to decide. This stage keeps one such lookup from
//! stalling everything behind it:
//!
//! ```text
//!             ┌─ decision known without I/O ──────────────▶ decided at once
//!   verified ─┤
//!             └─ unknown ─▶ park ─▶ ≤ max_concurrent lookups ─▶ decided later
//! ```
//!
//! - **Cache hits are immediate.** [`AuthPolicy::try_authorize`] is consulted
//!   first and never waits behind outstanding lookups.
//! - **Singleflight.** Messages from a sensor that already has a lookup parked
//!   or running join it; one chain query decides them all, in arrival order.
//! - **Bounded concurrency.** At most [`AuthLimits::max_concurrent`] lookups run
//!   at the same time, each in a task of a fixed-size [`JoinSet`]; no task is
//!   spawned for a message that is parked behind them.
//! - **Bounded memory, no shedding.** At most [`AuthLimits::max_pending`] messages
//!   are parked (running lookups included). A message that finds the stage full
//!   is handed back as [`Admission::Overflow`] and the pipeline holds it until a
//!   lookup finishes instead of dropping it. Ingress is not drained meanwhile, so
//!   its bounded channel fills and the transports push back on senders (HTTP
//!   answers `429`, which is retryable) rather than acknowledging telemetry that
//!   is later discarded. The cost is that cached sensors queue behind the
//!   saturated stage for at most one lookup. Dropping the stage aborts every
//!   running lookup, so shutdown has nothing left to wait for.
//!
//! The stage only decides; it never forwards. The pipeline sends a message on
//! to deduplication and the sinks only after a positive decision came back for
//! it here.

use super::Verified;
use crate::auth::{AuthError, AuthPolicy};
use crate::observability::metrics::{AUTH_LOOKUPS_IN_FLIGHT, AUTH_PENDING};
use crate::protocol::SensorId;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::task::{Id, JoinSet};

/// Default for [`AuthLimits::max_concurrent`].
pub const DEFAULT_MAX_CONCURRENT_LOOKUPS: usize = 8;

/// Default for [`AuthLimits::max_pending`].
pub const DEFAULT_MAX_PENDING: usize = 256;

/// Bounds of the authorization stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthLimits {
    /// Most lookups running at the same time (at least 1).
    pub max_concurrent: usize,
    /// Most messages parked waiting for a lookup, running ones included
    /// (at least 1).
    pub max_pending: usize,
}

impl Default for AuthLimits {
    fn default() -> Self {
        Self {
            max_concurrent: DEFAULT_MAX_CONCURRENT_LOOKUPS,
            max_pending: DEFAULT_MAX_PENDING,
        }
    }
}

/// What the stage did with a message handed to [`AuthStage::admit`].
pub(super) enum Admission {
    /// The decision was known without a lookup.
    Decided(bool, Verified),
    /// The message waits for a lookup; it comes back from
    /// [`AuthStage::next_resolution`].
    Parked,
    /// Too many messages are waiting; the message is given back unhandled so the
    /// caller can apply backpressure.
    Overflow(Verified),
}

/// The outcome of one lookup, for every message that waited for it.
pub(super) struct Resolution {
    /// The decision, or why none could be made.
    pub decision: Result<bool, AuthError>,
    /// The messages of that sensor in arrival order.
    pub messages: Vec<Verified>,
}

/// Runs the lookups of messages whose decision is not known yet.
pub(super) struct AuthStage {
    policy: Arc<dyn AuthPolicy>,
    limits: AuthLimits,
    /// Messages per sensor, waiting for or running a lookup.
    parked: HashMap<SensorId, Vec<Verified>>,
    /// Sensors in `parked` whose lookup has not started, oldest first.
    queued: VecDeque<SensorId>,
    /// The running lookups; never more than `limits.max_concurrent`.
    running: JoinSet<(SensorId, Result<bool, AuthError>)>,
    /// Which sensor a running task decides, to recover from a panicked task.
    running_ids: HashMap<Id, SensorId>,
    /// Messages in `parked`.
    pending: usize,
}

impl AuthStage {
    /// A stage consulting `policy` within `limits`.
    pub(super) fn new(policy: Arc<dyn AuthPolicy>, limits: AuthLimits) -> Self {
        Self {
            policy,
            limits: AuthLimits {
                max_concurrent: limits.max_concurrent.max(1),
                max_pending: limits.max_pending.max(1),
            },
            parked: HashMap::new(),
            queued: VecDeque::new(),
            running: JoinSet::new(),
            running_ids: HashMap::new(),
            pending: 0,
        }
    }

    /// Whether any message is waiting for a decision.
    pub(super) fn is_busy(&self) -> bool {
        self.pending > 0
    }

    /// Decide `message` now if possible, otherwise park it for a lookup.
    pub(super) fn admit(&mut self, message: Verified) -> Admission {
        if let Some(allowed) = self.policy.try_authorize(&message.verified.sensor_id) {
            return Admission::Decided(allowed, message);
        }
        if self.pending >= self.limits.max_pending {
            return Admission::Overflow(message);
        }

        self.pending += 1;
        let sensor_id = message.verified.sensor_id.clone();
        if let Some(waiting) = self.parked.get_mut(&sensor_id) {
            waiting.push(message);
        } else {
            self.parked.insert(sensor_id.clone(), vec![message]);
            self.queued.push_back(sensor_id);
            self.start_queued();
        }
        self.publish();
        Admission::Parked
    }

    /// The next finished lookup. Waits forever if none is running, so callers
    /// guard it with [`is_busy`](Self::is_busy). Cancel safe.
    pub(super) async fn next_resolution(&mut self) -> Resolution {
        let (sensor_id, decision) = match self.running.join_next_with_id().await {
            Some(Ok((id, finished))) => {
                self.running_ids.remove(&id);
                finished
            }
            Some(Err(error)) => {
                let Some(sensor_id) = self.running_ids.remove(&error.id()) else {
                    return std::future::pending().await;
                };
                tracing::error!(%error, %sensor_id, "authorization lookup task failed");
                (
                    sensor_id,
                    Err(AuthError::Lookup("lookup task failed".to_string())),
                )
            }
            None => return std::future::pending().await,
        };

        let messages = self.parked.remove(&sensor_id).unwrap_or_default();
        self.pending -= messages.len();
        self.start_queued();
        self.publish();
        Resolution { decision, messages }
    }

    /// Start lookups for queued sensors while there is room.
    fn start_queued(&mut self) {
        while self.running.len() < self.limits.max_concurrent {
            let Some(sensor_id) = self.queued.pop_front() else {
                break;
            };
            let policy = self.policy.clone();
            let lookup = sensor_id.clone();
            let handle = self.running.spawn(async move {
                let decision = policy.authorize(&lookup).await;
                (lookup, decision)
            });
            self.running_ids.insert(handle.id(), sensor_id);
        }
    }

    fn publish(&self) {
        metrics::gauge!(AUTH_PENDING).set(self.pending as f64);
        metrics::gauge!(AUTH_LOOKUPS_IN_FLIGHT).set(self.running.len() as f64);
    }
}

impl Drop for AuthStage {
    fn drop(&mut self) {
        metrics::gauge!(AUTH_PENDING).set(0.0);
        metrics::gauge!(AUTH_LOOKUPS_IN_FLIGHT).set(0.0);
    }
}
