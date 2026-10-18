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
//! Configuration of the embedded light node.

use super::error::{LightError, Result};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Default listen address: loopback, any free port.
const DEFAULT_LISTEN: &str = "127.0.0.1:0";

/// Configuration of a [`LightNode`](super::LightNode).
///
/// ```
/// use libcps::light::LightNodeConfig;
///
/// let config = LightNodeConfig::default().listen("127.0.0.1:9955");
/// # let _ = config;
/// ```
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LightNodeConfig {
    pub(crate) listen: String,
    pub(crate) allow_non_loopback: bool,
    pub(crate) relay_chain_spec: Option<String>,
    pub(crate) parachain_spec: Option<String>,
    pub(crate) max_connections: usize,
    pub(crate) max_pending_requests: u32,
    pub(crate) max_subscriptions: u32,
    pub(crate) max_message_size: usize,
}

impl Default for LightNodeConfig {
    fn default() -> Self {
        Self {
            listen: DEFAULT_LISTEN.to_string(),
            allow_non_loopback: false,
            relay_chain_spec: None,
            parachain_spec: None,
            max_connections: 32,
            max_pending_requests: 128,
            max_subscriptions: 1024,
            max_message_size: 8 * 1024 * 1024,
        }
    }
}

impl LightNodeConfig {
    /// Address of the local WebSocket JSON-RPC endpoint, e.g. `127.0.0.1:9944`.
    ///
    /// Port `0` lets the OS pick a free port; read it back with
    /// [`LightNode::rpc_addr`](super::LightNode::rpc_addr). Default: `127.0.0.1:0`.
    /// Non-loopback addresses are rejected unless
    /// [`allow_non_loopback`](Self::allow_non_loopback) is set.
    pub fn listen(mut self, addr: impl ToString) -> Self {
        self.listen = addr.to_string();
        self
    }

    /// Allow listening on a non-loopback address.
    ///
    /// The endpoint has no authentication; only enable this on trusted networks.
    pub fn allow_non_loopback(mut self, allow: bool) -> Self {
        self.allow_non_loopback = allow;
        self
    }

    /// Use custom chain specifications instead of the embedded Polkadot relay
    /// chain and Robonomics parachain ones.
    pub fn chain_specs(
        mut self,
        relay_chain: impl Into<String>,
        parachain: impl Into<String>,
    ) -> Self {
        self.relay_chain_spec = Some(relay_chain.into());
        self.parachain_spec = Some(parachain.into());
        self
    }

    /// Maximum number of simultaneously connected RPC clients (default 32).
    pub fn max_connections(mut self, max: usize) -> Self {
        self.max_connections = max;
        self
    }

    /// Maximum number of queued JSON-RPC requests per client (default 128).
    pub fn max_pending_requests(mut self, max: u32) -> Self {
        self.max_pending_requests = max;
        self
    }

    /// Maximum number of active subscriptions per client (default 1024).
    pub fn max_subscriptions(mut self, max: u32) -> Self {
        self.max_subscriptions = max;
        self
    }

    /// Maximum size in bytes of one WebSocket message (default 8 MiB).
    pub fn max_message_size(mut self, max: usize) -> Self {
        self.max_message_size = max;
        self
    }

    /// Parse and validate the listen address.
    pub(crate) fn listen_addr(&self) -> Result<SocketAddr> {
        let addr: SocketAddr = self
            .listen
            .parse()
            .map_err(|e| LightError::Config(format!("listen address `{}`: {e}", self.listen)))?;
        if !addr.ip().is_loopback() && !self.allow_non_loopback {
            return Err(LightError::Config(format!(
                "listen address {addr} is not loopback; enable `allow_non_loopback` to expose \
                 the unauthenticated light node endpoint"
            )));
        }
        if self.max_connections == 0 || self.max_pending_requests == 0 || self.max_message_size == 0
        {
            return Err(LightError::Config("RPC limits must be non-zero".into()));
        }
        Ok(addr)
    }

    /// Relay chain and parachain specifications to run.
    pub(crate) fn specs(&self) -> (&str, &str) {
        (
            self.relay_chain_spec
                .as_deref()
                .unwrap_or(robonomics_chain_spec::POLKADOT_RELAY_RAW),
            self.parachain_spec
                .as_deref()
                .unwrap_or(robonomics_chain_spec::POLKADOT_PARACHAIN_RAW),
        )
    }
}

/// URL clients should use to reach the endpoint bound at `addr`.
pub(crate) fn rpc_url(addr: SocketAddr) -> String {
    let ip = if addr.ip().is_unspecified() {
        match addr.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        }
    } else {
        addr.ip()
    };
    format!("ws://{}", SocketAddr::new(ip, addr.port()))
}
