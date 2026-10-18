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
//! Anchor tuning: retry policy and chain connection settings.

use libcps::NodeId;
use std::time::Duration;

/// Exponential back-off between retries of a transient failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Delay before the first retry.
    pub min: Duration,
    /// Upper bound the delay grows to.
    pub max: Duration,
}

impl RetryPolicy {
    /// Delay before retry number `attempt` (0-based): `min * 2^attempt`,
    /// capped at `max`.
    pub fn delay(&self, attempt: u32) -> Duration {
        let factor = 1u32.checked_shl(attempt.min(16)).unwrap_or(u32::MAX);
        self.min.saturating_mul(factor).min(self.max)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            min: Duration::from_secs(1),
            max: Duration::from_secs(60),
        }
    }
}

/// Everything needed to reach the chain and sign anchoring transactions.
#[derive(Clone)]
pub struct ChainConfig {
    /// RPC endpoint (`ws://` / `wss://`); `None` starts an embedded light node on a free local port.
    pub endpoint: Option<String>,
    /// Secret URI of the signing account. Never logged.
    pub suri: String,
    /// CPS node whose payload is updated.
    pub node: NodeId,
    /// Retry back-off for transient failures.
    pub retry: RetryPolicy,
    /// Largest payload the anchor will submit (defensive check only).
    pub max_payload: usize,
}

impl std::fmt::Debug for ChainConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChainConfig")
            .field("endpoint", &self.endpoint)
            .field("suri", &"<redacted>")
            .field("node", &self.node)
            .field("retry", &self.retry)
            .field("max_payload", &self.max_payload)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_and_caps() {
        let policy = RetryPolicy {
            min: Duration::from_secs(1),
            max: Duration::from_secs(10),
        };
        let delays: Vec<_> = (0..6).map(|n| policy.delay(n).as_secs()).collect();
        assert_eq!(delays, vec![1, 2, 4, 8, 10, 10]);
        assert_eq!(policy.delay(u32::MAX), Duration::from_secs(10));
    }

    #[test]
    fn debug_redacts_the_secret() {
        let config = ChainConfig {
            endpoint: None,
            suri: "//Alice".to_string(),
            node: NodeId(1),
            retry: RetryPolicy::default(),
            max_payload: 8192,
        };
        assert!(!format!("{config:?}").contains("Alice"));
    }
}
