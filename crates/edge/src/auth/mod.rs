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
//! Authorization policies applied *after* cryptographic verification.
//!
//! Authorization is always a decision about a verified [`SensorId`] and never
//! about transport metadata. A policy is consulted only once an envelope's
//! Ed25519 signature has been checked, so a policy can trust the identity it is
//! given.
//!
//! Three policies are provided:
//! - [`NoneAuth`]: accept every verified sensor (open gateway).
//! - [`Whitelist`]: accept only sensors present in a configured allow-list.
//! - [`CpsAuth`]: accept only sensors below a CPS root node, looked up on chain
//!   and kept in a bounded in-memory cache.

mod cps;
mod whitelist;

pub use cps::{CpsAuth, CpsReader, CpsSnapshot, Lineage};
pub use whitelist::Whitelist;

use crate::chain::ChainRegistry;
use crate::config::{AuthConfig, AuthMode};
use crate::protocol::SensorId;
use async_trait::async_trait;
use std::sync::Arc;

/// Why an authorization decision could not be made.
///
/// These are *indeterminate* outcomes, distinct from a denial: the sensor is
/// not let through, but nothing is remembered about it.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// The chain could not be reached (or is not synchronized yet).
    #[error("chain unavailable: {0}")]
    Unavailable(String),
    /// A chain query failed.
    #[error("CPS lookup failed: {0}")]
    Lookup(String),
    /// The lookup did not finish in time.
    #[error("CPS lookup timed out")]
    Timeout,
    /// The chain is not the one the gateway is configured for.
    #[error("unexpected genesis hash 0x{actual}, expected 0x{expected}")]
    ChainMismatch {
        /// Configured genesis hash, hex encoded.
        expected: String,
        /// Genesis hash reported by the chain, hex encoded.
        actual: String,
    },
    /// The configured root node does not exist on the chain.
    #[error("configured CPS root node {0} does not exist")]
    RootMissing(libcps::NodeId),
    /// The tree is corrupted (missing node, cycle, implausible depth).
    #[error("inconsistent CPS tree: {0}")]
    Inconsistent(String),
}

/// A pluggable authorization decision over a verified [`SensorId`].
///
/// [`authorize`] is called on the hot path for every verified envelope, so
/// implementations must be cheap in the common case. It may await (the CPS
/// policy queries the chain on a cache miss) but must be bounded in time.
///
/// The decision is `Ok(true)` / `Ok(false)`; `Err` means no decision could be
/// made and the envelope must be dropped.
///
/// [`authorize`]: AuthPolicy::authorize
#[async_trait]
pub trait AuthPolicy: Send + Sync {
    /// The configuration [`AuthMode`] this policy implements.
    fn mode(&self) -> AuthMode;

    /// The decision for `sensor_id` if it is known without any I/O.
    ///
    /// The pipeline asks this first and answers such sensors immediately; only
    /// when it returns `None` is the (possibly slow) [`authorize`] queued, so a
    /// slow lookup for one sensor never delays sensors whose decision is known.
    /// The default knows nothing.
    ///
    /// [`authorize`]: AuthPolicy::authorize
    fn try_authorize(&self, _sensor_id: &SensorId) -> Option<bool> {
        None
    }

    /// Decide whether the (already verified) `sensor_id` is authorized.
    async fn authorize(&self, sensor_id: &SensorId) -> Result<bool, AuthError>;
}

/// The open policy: every cryptographically verified sensor is authorized.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoneAuth;

#[async_trait]
impl AuthPolicy for NoneAuth {
    fn mode(&self) -> AuthMode {
        AuthMode::None
    }

    fn try_authorize(&self, _sensor_id: &SensorId) -> Option<bool> {
        Some(true)
    }

    async fn authorize(&self, _sensor_id: &SensorId) -> Result<bool, AuthError> {
        Ok(true)
    }
}

/// Build the configured authorization policy.
///
/// For [`AuthMode::Whitelist`] a whitelist file must be configured
/// (`[auth] file`); it is read eagerly so a missing or malformed file fails at
/// startup rather than silently rejecting every message.
///
/// For [`AuthMode::Cps`] the chain at `endpoint` (`None`: embedded light client)
/// is read through `chains` and connected on first use, so the policy is built
/// without touching the network.
pub fn from_config(
    config: &AuthConfig,
    endpoint: Option<&str>,
    chains: Arc<ChainRegistry>,
) -> std::io::Result<Box<dyn AuthPolicy>> {
    match config.mode {
        AuthMode::None => Ok(Box::new(NoneAuth)),
        AuthMode::Whitelist => {
            let path = config.file.as_ref().ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "auth mode 'whitelist' requires '[auth] file'",
                )
            })?;
            Ok(Box::new(Whitelist::from_file(path)?))
        }
        AuthMode::Cps => {
            let policy = CpsAuth::from_config(config, endpoint, chains)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            Ok(Box::new(policy))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SensorIdentity;

    fn sensor(seed: u8) -> SensorId {
        SensorIdentity::from_secret_bytes(&[seed; 32]).sensor_id()
    }

    #[tokio::test]
    async fn none_authorizes_everyone() {
        let policy = NoneAuth;
        assert_eq!(policy.mode(), AuthMode::None);
        assert!(policy.authorize(&sensor(1)).await.unwrap());
        assert!(policy.authorize(&sensor(255)).await.unwrap());
    }

    #[test]
    fn from_config_none_builds_none() {
        let policy =
            from_config(&AuthConfig::default(), None, Arc::new(ChainRegistry::new())).unwrap();
        assert_eq!(policy.mode(), AuthMode::None);
    }

    #[test]
    fn from_config_whitelist_requires_file() {
        let config = AuthConfig {
            mode: AuthMode::Whitelist,
            ..AuthConfig::default()
        };
        assert!(from_config(&config, None, Arc::new(ChainRegistry::new())).is_err());
    }

    #[test]
    fn from_config_cps_requires_root_and_genesis() {
        let config = AuthConfig {
            mode: AuthMode::Cps,
            ..AuthConfig::default()
        };
        assert!(from_config(&config, None, Arc::new(ChainRegistry::new())).is_err());

        let config = AuthConfig {
            mode: AuthMode::Cps,
            cps_root_node_id: Some(1),
            cps_expected_genesis_hash: Some(format!("0x{}", "00".repeat(32))),
            ..AuthConfig::default()
        };
        let policy = from_config(&config, None, Arc::new(ChainRegistry::new())).unwrap();
        assert_eq!(policy.mode(), AuthMode::Cps);
    }
}
