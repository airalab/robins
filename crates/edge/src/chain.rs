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
//! [`ChainRegistry`] hands out one lazily established [`Chain`] per endpoint
//! (`None` being the embedded light client), so every component that asks for
//! the same endpoint shares a single connection.
//!
//! Connections are established on first use and never at daemon startup, so an
//! unreachable chain cannot block ingestion. A failed attempt is not
//! remembered: the next caller simply tries again.

use libcps::light::{LightError, LightNode, LightNodeConfig};
use libcps::{Cps, Error as CpsError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::OnceCell;

/// Failure to establish a chain connection.
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    /// The chain is temporarily unreachable; trying again may succeed.
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
    _light: Option<LightNode>,
}

impl Chain {
    /// Connect to `endpoint`, or start an embedded [`LightNode`] on a free
    /// loopback port when `endpoint` is `None`. The light node lives as long as
    /// the returned [`Chain`].
    pub async fn connect(endpoint: Option<&str>) -> Result<Self, ChainError> {
        let (light, url) = match endpoint {
            Some(url) => (None, url.to_string()),
            None => {
                let light = LightNode::start(LightNodeConfig::default())
                    .await
                    .map_err(classify_light)?;
                let url = light.rpc_url().to_string();
                (Some(light), url)
            }
        };
        let cps = Cps::connect(&url).await.map_err(classify)?;
        Ok(Self { cps, _light: light })
    }

    /// The `libcps` client.
    pub fn cps(&self) -> &Cps {
        &self.cps
    }
}

/// One lazily filled connection slot.
type Slot = Arc<OnceCell<Arc<Chain>>>;

/// Lazily established [`Chain`] connections, one per RPC endpoint.
#[derive(Default)]
pub struct ChainRegistry {
    chains: Mutex<HashMap<Option<String>, Slot>>,
}

impl ChainRegistry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// The shared connection for `endpoint`, connecting on first use.
    ///
    /// Concurrent callers wait for one connection attempt. A failed attempt is
    /// not cached.
    pub async fn get(&self, endpoint: Option<&str>) -> Result<Arc<Chain>, ChainError> {
        let cell = {
            let mut chains = self.chains.lock().unwrap_or_else(|e| e.into_inner());
            chains
                .entry(endpoint.map(str::to_owned))
                .or_default()
                .clone()
        };
        cell.get_or_try_init(|| async { Chain::connect(endpoint).await.map(Arc::new) })
            .await
            .cloned()
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
