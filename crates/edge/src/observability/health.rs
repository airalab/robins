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
//! Liveness and readiness state for the operations server.
//!
//! The distinction mirrors Kubernetes probes:
//!
//! - **Liveness** (`/health`): the process is running and its event loop has not
//!   wedged. It is `true` for the whole lifetime of a started daemon; a failure
//!   here should trigger a restart.
//! - **Readiness** (`/ready`): the daemon is ready to accept and forward
//!   traffic. Subsystems (e.g. the GossipSub publisher reaching its minimum peer
//!   count) flip this to `true` once their preconditions are met, and back to
//!   `false` while degraded, so a load balancer can route around the instance
//!   without killing it.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// A cheap, cloneable handle to the shared liveness/readiness state.
///
/// All clones observe the same underlying flags, so any subsystem can update
/// readiness while the operations server reads it.
#[derive(Clone, Debug)]
pub struct Health {
    inner: Arc<Inner>,
}

/// Shared interior of a [`Health`] handle.
#[derive(Debug)]
struct Inner {
    /// Whether the process is alive (set once the daemon has started).
    live: AtomicBool,
    /// Whether the daemon is ready to serve traffic.
    ready: AtomicBool,
}

impl Health {
    /// Create a handle that is live but **not yet ready**.
    ///
    /// Readiness starts `false` so orchestration waits for subsystems to signal
    /// they are up before routing traffic.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                live: AtomicBool::new(true),
                ready: AtomicBool::new(false),
            }),
        }
    }

    /// Whether the process is live.
    pub fn is_live(&self) -> bool {
        self.inner.live.load(Ordering::Relaxed)
    }

    /// Whether the daemon is ready to serve traffic.
    pub fn is_ready(&self) -> bool {
        self.inner.ready.load(Ordering::Relaxed)
    }

    /// Mark the daemon as live (`true`) or wedged (`false`).
    pub fn set_live(&self, live: bool) {
        self.inner.live.store(live, Ordering::Relaxed);
    }

    /// Mark the daemon as ready (`true`) or degraded (`false`).
    pub fn set_ready(&self, ready: bool) {
        self.inner.ready.store(ready, Ordering::Relaxed);
    }
}

impl Default for Health {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_live_but_not_ready() {
        let health = Health::new();
        assert!(health.is_live());
        assert!(!health.is_ready());
    }

    #[test]
    fn clones_share_state() {
        let health = Health::new();
        let clone = health.clone();
        health.set_ready(true);
        assert!(clone.is_ready());
        clone.set_ready(false);
        assert!(!health.is_ready());
    }

    #[test]
    fn liveness_can_be_cleared() {
        let health = Health::new();
        health.set_live(false);
        assert!(!health.is_live());
    }
}
