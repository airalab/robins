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
//! Two policies are provided for the MVP:
//! - [`NoneAuth`]: accept every verified sensor (open gateway).
//! - [`Whitelist`]: accept only sensors present in a configured allow-list.

mod whitelist;

pub use whitelist::Whitelist;

use crate::config::{AuthConfig, AuthMode};
use crate::protocol::SensorId;

/// A pluggable authorization decision over a verified [`SensorId`].
///
/// Implementations must be cheap and side-effect free: [`authorize`] is called
/// on the hot path for every verified envelope.
///
/// [`authorize`]: AuthPolicy::authorize
pub trait AuthPolicy: Send + Sync {
    /// The configuration [`AuthMode`] this policy implements.
    fn mode(&self) -> AuthMode;

    /// Return `true` if the (already verified) `sensor_id` is authorized.
    fn authorize(&self, sensor_id: &SensorId) -> bool;
}

/// The open policy: every cryptographically verified sensor is authorized.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoneAuth;

impl AuthPolicy for NoneAuth {
    fn mode(&self) -> AuthMode {
        AuthMode::None
    }

    fn authorize(&self, _sensor_id: &SensorId) -> bool {
        true
    }
}

/// Build the configured authorization policy.
///
/// For [`AuthMode::Whitelist`] a whitelist file must be configured
/// (`[auth] file`); it is read eagerly so a missing or malformed file fails at
/// startup rather than silently rejecting every message.
pub fn from_config(config: &AuthConfig) -> std::io::Result<Box<dyn AuthPolicy>> {
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SensorIdentity;

    fn sensor(seed: u8) -> SensorId {
        SensorIdentity::from_secret_bytes(&[seed; 32]).sensor_id()
    }

    #[test]
    fn none_authorizes_everyone() {
        let policy = NoneAuth;
        assert_eq!(policy.mode(), AuthMode::None);
        assert!(policy.authorize(&sensor(1)));
        assert!(policy.authorize(&sensor(255)));
    }

    #[test]
    fn from_config_none_builds_none() {
        let policy = from_config(&AuthConfig::default()).unwrap();
        assert_eq!(policy.mode(), AuthMode::None);
    }

    #[test]
    fn from_config_whitelist_requires_file() {
        let config = AuthConfig {
            mode: AuthMode::Whitelist,
            file: None,
        };
        assert!(from_config(&config).is_err());
    }
}
