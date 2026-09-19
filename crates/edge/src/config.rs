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
//! Gateway configuration (`gateway.toml`).
//!
//! `edge gw` is configuration-first: the daemon is driven by a TOML file whose
//! schema is defined here and mirrors the `edge` API specification. Sections for
//! subsystems that land in later phases (`ipfs`, `blockchain`, `storage`) are
//! modelled so that `edge config check` can validate a complete file, even
//! though the MVP runtime only wires up `http`, `meshtastic`, `auth`,
//! `pubsub` and `metrics`.
//!
//! Secrets are never embedded directly: token/key material is referenced through
//! `*_file` paths so it can be injected out-of-band and redacted by
//! `edge config print`.

use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Default location of the gateway configuration file.
pub const DEFAULT_CONFIG_PATH: &str = "/etc/edge/gateway.toml";

/// Top-level gateway configuration, deserialized from `gateway.toml`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// HTTP ingress transport.
    pub http: HttpConfig,
    /// Authentication policy applied after signature verification.
    pub auth: AuthConfig,
    /// Native libp2p GossipSub publisher.
    pub pubsub: PubsubConfig,
    /// Meshtastic ingress transport.
    pub meshtastic: MeshtasticConfig,
    /// IPFS durable publishing (deferred subsystem).
    pub ipfs: IpfsConfig,
    /// Robonomics anchoring (deferred subsystem).
    pub blockchain: BlockchainConfig,
    /// Embedded persistent storage (deferred subsystem).
    pub storage: StorageConfig,
    /// Operations/metrics HTTP server.
    pub metrics: MetricsConfig,
    /// Internal pipeline sizing.
    pub pipeline: PipelineConfig,
}

/// HTTP ingress listener configuration (`[http]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpConfig {
    /// Whether the HTTP ingress transport is enabled.
    pub enabled: bool,
    /// Address to bind the ingress listener to.
    pub listen: SocketAddr,
    /// Maximum accepted request body size, in bytes.
    pub max_body_bytes: usize,
}

impl Default for HttpConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: "0.0.0.0:3000".parse().expect("valid default http listen"),
            // 64 KiB is generous for a single signed envelope.
            max_body_bytes: 64 * 1024,
        }
    }
}

/// Authentication mode applied after protocol/signature validation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthMode {
    /// Accept every envelope that passes cryptographic/protocol validation.
    #[default]
    None,
    /// Accept only envelopes whose `sensor_id` is in the whitelist.
    Whitelist,
}

/// Authentication policy configuration (`[auth]`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AuthConfig {
    /// Selected authentication mode.
    pub mode: AuthMode,
    /// Path to a whitelist file (one SS58 or `0x`-prefixed hex `sensor_id`
    /// per line). Required when [`AuthMode::Whitelist`] is selected.
    pub file: Option<PathBuf>,
}

/// libp2p GossipSub publisher configuration (`[pubsub]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PubsubConfig {
    /// Whether the GossipSub publisher is enabled.
    pub enabled: bool,
    /// Multiaddresses to listen on.
    pub listen: Vec<String>,
    /// GossipSub topic accepted messages are published to.
    pub topic: String,
    /// Reserved peers to dial and keep connected (multiaddrs).
    pub reserved_peers: Vec<String>,
    /// Minimum connected peers before the live-publish path is considered ready.
    pub min_connected_peers: usize,
    /// Path to the persisted libp2p Ed25519 identity key. Generated if absent.
    pub identity_file: Option<PathBuf>,
}

impl Default for PubsubConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            listen: vec!["/ip4/0.0.0.0/tcp/64442".to_string()],
            topic: "sensors.social/v1".to_string(),
            reserved_peers: Vec::new(),
            min_connected_peers: 0,
            identity_file: None,
        }
    }
}

/// Meshtastic ingress configuration (`[meshtastic]`).
///
/// Mirrors the receive-side parameters of the Connectivity Protocol
/// Meshtastic Transport v1 specification
/// (`src/protobufs/transport/meshtastic/v1.md`). The MVP only supports the
/// `serial` transport kind; BLE and TCP radios are out of scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MeshtasticConfig {
    /// Whether Meshtastic ingress is enabled.
    pub enabled: bool,
    /// Transport kind. Only `"serial"` is currently supported.
    pub transport: Option<String>,
    /// Serial device path (e.g. `/dev/ttyACM0`).
    pub device: Option<String>,
    /// Connectivity Protocol Meshtastic application `PortNum`.
    ///
    /// `PRIVATE_APP = 256` is appropriate for development; production
    /// deployments should use a registered third-party port in `64..127`.
    pub port_num: u32,
    /// Maximum bytes retained for one reassembled envelope. The wire
    /// protocol itself caps this at 3376 bytes (16 fragments * 211 bytes).
    pub max_reassembled_bytes: usize,
    /// Maximum fragments accepted for one envelope. Must not exceed the
    /// protocol limit of 16.
    pub max_fragments: usize,
    /// Maximum number of incomplete reassembly slots retained globally.
    pub max_pending: usize,
    /// Maximum number of incomplete reassembly slots retained per mesh
    /// sender.
    pub max_pending_per_sender: usize,
    /// Idle timeout for an incomplete assembly, in seconds: reset whenever a
    /// new (non-duplicate) fragment advances it.
    pub reassembly_timeout_secs: u64,
    /// Absolute lifetime for an incomplete assembly, in seconds, measured
    /// from creation and never extended.
    pub reassembly_absolute_timeout_secs: u64,
    /// Minimum serial reconnect backoff, in seconds.
    pub reconnect_min_secs: u64,
    /// Maximum serial reconnect backoff, in seconds.
    pub reconnect_max_secs: u64,
}

impl Default for MeshtasticConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            transport: Some("serial".to_string()),
            device: None,
            // PRIVATE_APP, per the Transport v1 spec §4.
            port_num: 256,
            max_reassembled_bytes: 4096,
            max_fragments: 16,
            max_pending: 128,
            max_pending_per_sender: 8,
            reassembly_timeout_secs: 60,
            reassembly_absolute_timeout_secs: 300,
            reconnect_min_secs: 1,
            reconnect_max_secs: 30,
        }
    }
}

/// IPFS durable publishing configuration (`[ipfs]`, deferred subsystem).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IpfsConfig {
    /// Whether durable IPFS publishing is enabled.
    pub enabled: bool,
    /// Ordered list of enabled provider names (e.g. `["pinata", "kubo"]`).
    pub providers: Vec<String>,
    /// Durability policy: `any`, `all`, or `quorum`.
    pub durability: Option<String>,
    /// Pinata provider settings.
    pub pinata: Option<PinataConfig>,
    /// Kubo provider settings.
    pub kubo: Option<KuboConfig>,
}

/// Pinata IPFS provider configuration (`[ipfs.pinata]`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PinataConfig {
    /// Path to a file containing the Pinata API token.
    pub token_file: Option<PathBuf>,
}

/// Kubo IPFS provider configuration (`[ipfs.kubo]`).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KuboConfig {
    /// Kubo/IPFS HTTP API URL.
    pub url: Option<String>,
}

/// Robonomics anchoring configuration (`[blockchain]`, deferred subsystem).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BlockchainConfig {
    /// Whether blockchain anchoring is enabled.
    pub enabled: bool,
    /// Robonomics RPC endpoint (websocket).
    pub endpoint: Option<String>,
    /// Path to a file containing the anchoring signing key.
    pub key_file: Option<PathBuf>,
}

/// Embedded persistent storage configuration (`[storage]`, deferred subsystem).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// Path to the embedded `redb` database file.
    pub path: Option<PathBuf>,
}

/// Operations/metrics HTTP server configuration (`[metrics]`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct MetricsConfig {
    /// Address for the operations server (`/health`, `/ready`, `/metrics`,
    /// `/debug/peers`).
    pub listen: SocketAddr,
}

impl Default for MetricsConfig {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:9090"
                .parse()
                .expect("valid default metrics listen"),
        }
    }
}

/// Internal pipeline sizing. All channels are bounded for backpressure.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineConfig {
    /// Capacity of the ingress -> pipeline channel.
    pub ingress_buffer: usize,
    /// Maximum number of message-id entries retained by the dedup cache.
    pub dedup_capacity: usize,
    /// Time-to-live for dedup entries, in seconds.
    pub dedup_ttl_secs: u64,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            ingress_buffer: 1024,
            dedup_capacity: 8192,
            dedup_ttl_secs: 300,
        }
    }
}

/// Errors that can occur while loading configuration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// The configuration file could not be read.
    #[error("failed to read config file {path}: {source}")]
    Read {
        /// Offending path.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The configuration file could not be parsed as TOML.
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
    /// The configuration is semantically invalid.
    #[error("invalid configuration: {0}")]
    Invalid(String),
}

impl Config {
    /// Loads and validates configuration from a TOML file at `path`.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Config = toml::from_str(&text)?;
        config.validate()?;
        Ok(config)
    }

    /// Validates cross-field invariants that serde cannot express.
    ///
    /// This backs `edge config check`: it catches missing required fields,
    /// conflicting settings, and provider misconfiguration.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.auth.mode == AuthMode::Whitelist && self.auth.file.is_none() {
            return Err(ConfigError::Invalid(
                "auth.mode = \"whitelist\" requires auth.file".to_string(),
            ));
        }

        if self.ipfs.enabled {
            if self.ipfs.providers.is_empty() {
                return Err(ConfigError::Invalid(
                    "ipfs.enabled = true requires at least one entry in ipfs.providers".to_string(),
                ));
            }
            for provider in &self.ipfs.providers {
                match provider.as_str() {
                    "pinata" | "kubo" => {}
                    other => {
                        return Err(ConfigError::Invalid(format!(
                            "unknown ipfs provider {other:?} (expected \"pinata\" or \"kubo\")"
                        )));
                    }
                }
            }
            if let Some(policy) = &self.ipfs.durability {
                match policy.as_str() {
                    "any" | "all" | "quorum" => {}
                    other => {
                        return Err(ConfigError::Invalid(format!(
                            "unknown ipfs.durability {other:?} (expected any|all|quorum)"
                        )));
                    }
                }
            }
        }

        if self.meshtastic.enabled {
            match self.meshtastic.transport.as_deref() {
                Some("serial") => {}
                Some(other) => {
                    return Err(ConfigError::Invalid(format!(
                        "unknown meshtastic.transport {other:?} (expected \"serial\")"
                    )));
                }
                None => {
                    return Err(ConfigError::Invalid(
                        "meshtastic.enabled = true requires meshtastic.transport".to_string(),
                    ));
                }
            }
            if self.meshtastic.device.is_none() {
                return Err(ConfigError::Invalid(
                    "meshtastic.enabled = true requires meshtastic.device".to_string(),
                ));
            }
            if self.meshtastic.max_reassembled_bytes == 0 {
                return Err(ConfigError::Invalid(
                    "meshtastic.max_reassembled_bytes must be non-zero".to_string(),
                ));
            }
            if self.meshtastic.max_fragments == 0
                || self.meshtastic.max_fragments > crate::ingress::meshtastic::MAX_FRAGMENT_COUNT
            {
                return Err(ConfigError::Invalid(format!(
                    "meshtastic.max_fragments must be in 1..={} (Transport v1 wire limit)",
                    crate::ingress::meshtastic::MAX_FRAGMENT_COUNT
                )));
            }
            if self.meshtastic.max_pending == 0 {
                return Err(ConfigError::Invalid(
                    "meshtastic.max_pending must be non-zero".to_string(),
                ));
            }
            if self.meshtastic.max_pending_per_sender == 0 {
                return Err(ConfigError::Invalid(
                    "meshtastic.max_pending_per_sender must be non-zero".to_string(),
                ));
            }
            if self.meshtastic.reassembly_timeout_secs == 0 {
                return Err(ConfigError::Invalid(
                    "meshtastic.reassembly_timeout_secs must be non-zero".to_string(),
                ));
            }
            if self.meshtastic.reassembly_absolute_timeout_secs == 0 {
                return Err(ConfigError::Invalid(
                    "meshtastic.reassembly_absolute_timeout_secs must be non-zero".to_string(),
                ));
            }
            if self.meshtastic.reconnect_min_secs == 0 {
                return Err(ConfigError::Invalid(
                    "meshtastic.reconnect_min_secs must be non-zero".to_string(),
                ));
            }
            if self.meshtastic.reconnect_max_secs < self.meshtastic.reconnect_min_secs {
                return Err(ConfigError::Invalid(
                    "meshtastic.reconnect_max_secs must be >= meshtastic.reconnect_min_secs"
                        .to_string(),
                ));
            }
        }

        if self.blockchain.enabled && self.blockchain.endpoint.is_none() {
            return Err(ConfigError::Invalid(
                "blockchain.enabled = true requires blockchain.endpoint".to_string(),
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        Config::default().validate().expect("defaults valid");
    }

    #[test]
    fn parses_spec_example() {
        let toml = r#"
            [auth]
            mode = "whitelist"
            file = "/etc/edge/whitelist.txt"

            [http]
            enabled = true
            listen = "0.0.0.0:8080"

            [pubsub]
            enabled = true
            topic = "sensors.social"
            min_connected_peers = 2
            reserved_peers = [
              "/dns4/peer1.example.org/tcp/4001/p2p/QmPeer1",
            ]

            [ipfs]
            enabled = true
            providers = ["pinata", "kubo"]
            durability = "any"

            [ipfs.pinata]
            token_file = "/etc/edge/secrets/pinata"

            [ipfs.kubo]
            url = "http://127.0.0.1:5001"

            [storage]
            path = "/var/lib/edge/state.redb"

            [metrics]
            listen = "127.0.0.1:9090"
        "#;
        let cfg: Config = toml::from_str(toml).expect("parse");
        cfg.validate().expect("valid");
        assert_eq!(cfg.auth.mode, AuthMode::Whitelist);
        assert_eq!(cfg.pubsub.min_connected_peers, 2);
        assert_eq!(cfg.ipfs.providers, vec!["pinata", "kubo"]);
        assert_eq!(cfg.metrics.listen.port(), 9090);
    }

    #[test]
    fn whitelist_without_file_is_rejected() {
        let cfg = Config {
            auth: AuthConfig {
                mode: AuthMode::Whitelist,
                file: None,
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn ipfs_enabled_requires_provider() {
        let cfg = Config {
            ipfs: IpfsConfig {
                enabled: true,
                providers: vec![],
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }
}
