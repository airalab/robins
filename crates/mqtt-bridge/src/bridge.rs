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
//! MQTT bridge implementation for connecting blockchain nodes to MQTT brokers.
//!
//! This module provides the core MQTT bridge functionality that can be used
//! both from the CLI and as a library. It supports bidirectional communication:
//! - **Subscribe mode**: Listen to MQTT topics and update blockchain node payloads
//! - **Publish mode**: Monitor blockchain events and publish to MQTT topics
//! - **Config file mode**: Manage multiple bridges from a TOML configuration file
//!
//! # Examples
//!
//! ## Basic Configuration
//!
//! ```
//! use mqtt_bridge::Config;
//!
//! let config = Config {
//!     broker: "mqtt://localhost:1883".to_string(),
//!     username: Some("user".to_string()),
//!     password: Some("pass".to_string()),
//!     client_id: Some("my-client".to_string()),
//!     blockchain: None,
//!     subscribe: Vec::new(),
//!     publish: Vec::new(),
//! };
//! ```
//!
//! ## Loading from Configuration File
//!
//! ```no_run
//! use mqtt_bridge::Config;
//!
//! # async fn example() -> anyhow::Result<()> {
//! // Load config from TOML file
//! let config = Config::from_file("mqtt_config.toml")?;
//!
//! // Start all configured bridges
//! config.start().await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Configuration File Format
//!
//! ```toml
//! broker = "mqtt://localhost:1883"
//! username = "myuser"
//! password = "mypass"
//!
//! [blockchain]
//! ws_url = "ws://localhost:9944"
//! suri = "//Alice"
//!
//! [[subscribe]]
//! topic = "sensors/temperature"
//! node_id = 5
//!
//! [[subscribe]]
//! topic = "sensors/humidity"
//! node_id = 6
//! receiver_public = "5GrwvaEF..."
//! cipher = "xchacha20"
//! scheme = "sr25519"
//!
//! [[publish]]
//! topic = "actuators/valve"
//! node_id = 10
//! ```
//!
//! ## Programmatic Usage
//!
//! ```no_run
//! use mqtt_bridge::BlockchainConfig;
//! use mqtt_bridge as mqtt;
//!
//! # async fn example() -> anyhow::Result<()> {
//! let blockchain_config = BlockchainConfig {
//!     ws_url: "ws://localhost:9944".to_string(),
//!     suri: Some("//Alice".to_string()),
//! };
//!
//! let mqtt_config = mqtt::Config {
//!     broker: "mqtt://localhost:1883".to_string(),
//!     username: None,
//!     password: None,
//!     client_id: None,
//!     blockchain: None,
//!     subscribe: Vec::new(),
//!     publish: Vec::new(),
//! };
//!
//! // Subscribe: MQTT -> Blockchain
//! mqtt_config.subscribe(
//!     &blockchain_config,
//!     None::<&libcps::crypto::Signer<libcps::crypto::Sr25519>>,
//!     "sensors/temp",
//!     1,
//!     None,
//!     None,
//!     None,
//! ).await?;
//!
//! // Publish: Blockchain -> MQTT
//! mqtt_config.publish(
//!     &blockchain_config,
//!     None::<&libcps::crypto::Signer<libcps::crypto::Sr25519>>,
//!     "actuators/status",
//!     1,
//!     None,
//! ).await?;
//! # Ok(())
//! # }
//! ```
//!
//! // Anonymous connection
//! let config = Config {
//!     broker: "mqtt://localhost:1883".to_string(),
//!     username: None,
//!     password: None,
//!     client_id: None,
//! };
//!
//! // Authenticated connection
//! let config_auth = Config {
//!     broker: "mqtt://broker.example.com:1883".to_string(),
//!     username: Some("myuser".to_string()),
//!     password: Some("mypass".to_string()),
//!     client_id: Some("cps-client".to_string()),
//! };
//! ```

use anyhow::{anyhow, Result};
use libcps::crypto::{
    Cipher, EncryptedMessage, EncryptionAlgorithm, PublicKey, Scheme, Signer, Sr25519,
};
use libcps::{prelude::*, Client, NodeId};
use parity_scale_codec::Decode;
use parity_scale_codec::Encode;
use rumqttc::{AsyncClient, Event, MqttOptions, Packet, QoS};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tokio::time::{sleep, Duration};
use tracing::{debug, error, trace};

/// Configuration for a subscribe topic
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubscribeConfig {
    /// MQTT topic to subscribe to
    pub topic: String,
    /// Node ID to update with received messages
    pub node_id: u64,
    /// Optional receiver public key for encryption (hex or SS58 format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receiver_public: Option<String>,
    /// Encryption cipher algorithm (xchacha20, aesgcm256, chacha20)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cipher: Option<String>,
    /// Cryptographic scheme (sr25519, ed25519)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheme: Option<String>,
}

/// Configuration for a publish topic
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishConfig {
    /// MQTT topic to publish to
    pub topic: String,
    /// Node ID to monitor for changes
    pub node_id: u64,
    /// Whether to decrypt encrypted blockchain payloads before publishing to MQTT
    /// Requires SURI to be configured in blockchain section
    #[serde(default, skip_serializing_if = "is_false")]
    pub decrypt: bool,
}

// Helper for serde skip_serializing_if
fn is_false(b: &bool) -> bool {
    !b
}

/// Blockchain connection settings used by the bridge functions.
#[derive(Clone, Debug)]
pub struct BlockchainConfig {
    /// WebSocket URL of the node RPC endpoint
    pub ws_url: String,
    /// Account secret URI (e.g., //Alice or seed phrase)
    pub suri: Option<String>,
}

/// Blockchain connection configuration
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlockchainConfigData {
    /// WebSocket URL for blockchain connection
    pub ws_url: String,
    /// Account secret URI (e.g., //Alice or seed phrase)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suri: Option<String>,
}

/// Configuration for MQTT broker connection.
///
/// This configuration is used to establish connections to MQTT brokers
/// for IoT device integration. It can be loaded from a TOML file or
/// created programmatically.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    /// MQTT broker URL (e.g., "mqtt://localhost:1883")
    pub broker: String,
    /// Optional username for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Optional password for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Optional client ID for MQTT connection
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Blockchain connection configuration (for config file usage)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blockchain: Option<BlockchainConfigData>,
    /// List of topics to subscribe to (for config file usage)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscribe: Vec<SubscribeConfig>,
    /// List of topics to publish to (for config file usage)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub publish: Vec<PublishConfig>,
}

impl Config {
    /// Load configuration from a TOML file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the TOML configuration file
    ///
    /// # Returns
    ///
    /// Returns the loaded Config or an error if the file cannot be read or parsed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mqtt_bridge::Config;
    /// # fn example() -> anyhow::Result<()> {
    /// let config = Config::from_file("config.toml")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_file(path: &str) -> Result<Self> {
        let contents = std::fs::read_to_string(path)
            .map_err(|e| anyhow!("Failed to read config file '{}': {}", path, e))?;

        let config: Config =
            toml::from_str(&contents).map_err(|e| anyhow!("Failed to parse TOML config: {}", e))?;

        Ok(config)
    }

    /// Start MQTT bridges for all configured topics.
    ///
    /// This method spawns concurrent tasks for all subscribe and publish topics
    /// defined in the configuration. It requires blockchain configuration to be
    /// present in the config.
    ///
    /// # Returns
    ///
    /// This function runs indefinitely and only returns on fatal errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mqtt_bridge::Config;
    /// # async fn example() -> anyhow::Result<()> {
    /// let config = Config::from_file("config.toml")?;
    /// config.start().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn start(&self) -> Result<()> {
        debug!("Starting MQTT bridge from configuration file");
        trace!(
            "Configuration: {} subscribe topics, {} publish topics",
            self.subscribe.len(),
            self.publish.len()
        );

        // Validate blockchain config exists
        let blockchain_data = self
            .blockchain
            .as_ref()
            .ok_or_else(|| anyhow!("Blockchain configuration required in config file"))?;

        debug!(
            "Blockchain config: ws_url={}, suri={}",
            blockchain_data.ws_url,
            if blockchain_data.suri.is_some() {
                "present"
            } else {
                "none"
            }
        );

        let blockchain_config = BlockchainConfig {
            ws_url: blockchain_data.ws_url.clone(),
            suri: blockchain_data.suri.clone(),
        };

        // Create a task set for concurrent execution
        let mut tasks = Vec::new();

        // Spawn subscribe tasks
        for sub in &self.subscribe {
            debug!(
                "Setting up subscribe task for topic '{}' -> node {}",
                sub.topic, sub.node_id
            );
            let blockchain_cfg = blockchain_config.clone();
            let mqtt_cfg = self.clone();
            let topic = sub.topic.clone();
            let node_id = sub.node_id;
            let receiver_public = sub.receiver_public.clone();
            let cipher_name = sub
                .cipher
                .clone()
                .unwrap_or_else(|| "xchacha20".to_string());
            let scheme_name = sub.scheme.clone().unwrap_or_else(|| "sr25519".to_string());

            let task = tokio::spawn(async move {
                trace!("Subscribe task starting for topic '{}'", topic);
                // Parse receiver public key if provided
                let receiver_pub_bytes = if let Some(ref addr_or_hex) = receiver_public {
                    debug!("Parsing receiver public key for encryption");
                    Some(parse_receiver_public_key(addr_or_hex)?)
                } else {
                    None
                };

                // Create cipher if encryption is requested
                let (scheme, algorithm_opt) = if receiver_public.is_some() {
                    debug!(
                        "Creating cipher with algorithm={}, scheme={}",
                        cipher_name, scheme_name
                    );
                    let algorithm = EncryptionAlgorithm::from_str(&cipher_name)
                        .map_err(|e| anyhow!("Invalid cipher '{}': {}", cipher_name, e))?;
                    let scheme = CryptoScheme::from_str(&scheme_name)
                        .map_err(|e| anyhow!("Invalid scheme '{}': {}", scheme_name, e))?;
                    (scheme, Some(algorithm))
                } else {
                    trace!("No encryption configured for this subscription");
                    (CryptoScheme::Sr25519, None)
                };

                // Start subscribe bridge
                crate::with_scheme!(scheme, S => {
                    let cipher: Option<Signer<S>> = if receiver_public.is_some() {
                        let suri = blockchain_cfg
                            .suri
                            .clone()
                            .ok_or_else(|| anyhow!("SURI required for encryption"))?;
                        Some(Signer::from_suri(&suri)?)
                    } else {
                        None
                    };

                    mqtt_cfg
                        .subscribe(
                            &blockchain_cfg,
                            cipher.as_ref(),
                            &topic,
                            node_id,
                            receiver_pub_bytes,
                            algorithm_opt,
                            None, // No custom message handler
                        )
                        .await
                })
            });

            tasks.push(task);
        }

        // Spawn publish tasks
        for pub_cfg in &self.publish {
            let blockchain_cfg = blockchain_config.clone();
            let mqtt_cfg = self.clone();
            let topic = pub_cfg.topic.clone();
            let node_id = pub_cfg.node_id;
            let should_decrypt = pub_cfg.decrypt;

            let task = tokio::spawn(async move {
                // Create cipher for decryption if requested
                // Note: Algorithm and scheme are auto-detected from encrypted data
                // We only need our private key (SURI) to create the Cipher
                let cipher: Option<Signer<Sr25519>> = if should_decrypt {
                    let suri = blockchain_cfg
                        .suri
                        .clone()
                        .ok_or_else(|| anyhow!("SURI required for decryption"))?;
                    // Use default scheme (SR25519) for Cipher creation
                    // The actual algorithm used will be read from the encrypted message
                    Some(Signer::from_suri(&suri)?)
                } else {
                    None
                };

                mqtt_cfg
                    .publish(
                        &blockchain_cfg,
                        cipher.as_ref(),
                        &topic,
                        node_id,
                        None, // No custom publish handler
                    )
                    .await
            });

            tasks.push(task);
        }

        // Wait for all tasks (they run indefinitely)
        for task in tasks {
            if let Err(e) = task.await {
                error!("Bridge task failed: {}", e);
            }
        }

        Ok(())
    }

    /// Subscribe to MQTT topic and update blockchain node payload with received messages.
    ///
    /// This method creates a long-running bridge that:
    /// 1. Connects to the specified MQTT broker
    /// 2. Subscribes to the given topic
    /// 3. On each message, optionally encrypts it and updates the blockchain node payload
    /// 4. Automatically reconnects on connection failures
    ///
    /// # Arguments
    ///
    /// * `blockchain_config` - Configuration for blockchain connection
    /// * `cipher` - Optional cipher for encrypting messages before sending to blockchain
    /// * `topic` - MQTT topic to subscribe to
    /// * `node_id` - Blockchain node ID to update
    /// * `receiver_public` - Optional public key for encryption (required if cipher is provided)
    /// * `algorithm` - Optional encryption algorithm (required if cipher is provided)
    /// * `message_handler` - Optional callback for custom message processing
    ///
    /// # Returns
    ///
    /// This function runs indefinitely and only returns on fatal errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mqtt_bridge::BlockchainConfig;
    /// # use mqtt_bridge as mqtt;
    /// # async fn example() -> anyhow::Result<()> {
    /// let blockchain_config = BlockchainConfig {
    ///     ws_url: "ws://localhost:9944".to_string(),
    ///     suri: Some("//Alice".to_string()),
    /// };
    ///
    /// let mqtt_config = mqtt::Config {
    ///     broker: "mqtt://localhost:1883".to_string(),
    ///     username: None,
    ///     password: None,
    ///     client_id: None,
    ///     blockchain: None,
    ///     subscribe: Vec::new(),
    ///     publish: Vec::new(),
    /// };
    ///
    /// mqtt_config.subscribe(
    ///     &blockchain_config,
    ///     None::<&libcps::crypto::Signer<libcps::crypto::Sr25519>>,
    ///     "sensors/temp",
    ///     1,
    ///     None,
    ///     None,
    ///     None,
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn subscribe<S: Scheme>(
        &self,
        blockchain_config: &BlockchainConfig,
        cipher: Option<&Signer<S>>,
        topic: &str,
        node_id: u64,
        receiver_public: Option<[u8; 32]>,
        algorithm: Option<EncryptionAlgorithm>,
        message_handler: Option<MessageHandler>,
    ) -> Result<()> {
        debug!(
            "Starting MQTT subscribe bridge: topic='{}', node={}",
            topic, node_id
        );
        debug!(
            "Subscribe config: encryption={}, algorithm={:?}",
            receiver_public.is_some(),
            algorithm
        );

        // Connect to blockchain
        trace!("Connecting to blockchain at {}", blockchain_config.ws_url);
        let client = Client::connect(Some(&blockchain_config.ws_url)).await?;
        let signer = signer(blockchain_config)?;
        debug!("Connected to blockchain successfully");

        // Parse MQTT broker URL
        let (host, port) = parse_mqtt_url(&self.broker)?;
        debug!("MQTT broker: {}:{}", host, port);

        // Configure MQTT client
        let client_id = self
            .client_id
            .clone()
            .unwrap_or_else(|| format!("cps-sub-{}", node_id));

        trace!("MQTT client_id: {}", client_id);
        let mut mqttoptions = MqttOptions::new(client_id, host, port);
        mqttoptions.set_keep_alive(Duration::from_secs(30));

        // Set credentials if provided
        if let Some(username) = &self.username {
            mqttoptions.set_credentials(username, self.password.as_deref().unwrap_or(""));
        }

        // Create MQTT client with eventloop
        let (mqtt_client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

        // Subscribe to topic
        mqtt_client
            .subscribe(topic, QoS::AtMostOnce)
            .await
            .map_err(|e| anyhow!("Failed to subscribe to topic: {}", e))?;

        let node = NodeId::from(node_id);

        // Process MQTT events
        debug!("Starting MQTT event loop for topic '{}'", topic);
        loop {
            match eventloop.poll().await {
                Ok(Event::Incoming(Packet::Publish(publish))) => {
                    trace!(
                        "Received MQTT message on topic '{}': {} bytes",
                        topic,
                        publish.payload.len()
                    );

                    // Call custom handler if provided
                    if let Some(ref handler) = message_handler {
                        handler(topic, &publish.payload);
                    }

                    // Prepare node data (encrypt if needed)
                    let node_data = match (receiver_public.as_ref(), cipher, algorithm) {
                        (Some(receiver_pub), Some(cipher), Some(algorithm)) => {
                            debug!("Encrypting message with {:?} algorithm", algorithm);
                            let receiver = PublicKey::<S>::from(*receiver_pub);
                            match cipher.encrypt(&publish.payload, &receiver, algorithm) {
                                Ok(encrypted_message) => {
                                    let encrypted_bytes = encrypted_message.encode();
                                    trace!(
                                        "Encrypted message: {} bytes -> {} bytes",
                                        publish.payload.len(),
                                        encrypted_bytes.len()
                                    );
                                    encrypted_bytes
                                }
                                Err(e) => {
                                    // Encryption failed, log error and continue
                                    error!("Failed to encrypt message: {}", e);
                                    continue;
                                }
                            }
                        }
                        (Some(_), _, _) => {
                            // Invalid bridge configuration: receiver_public is set but cipher or algorithm is missing
                            unreachable!("invalid bridge configuration: receiver_public is set but cipher or algorithm is missing");
                        }
                        _ => {
                            let payload_str = String::from_utf8_lossy(&publish.payload);
                            trace!("Using plaintext payload");
                            payload_str.as_bytes().to_vec()
                        }
                    };

                    // Update node payload on blockchain
                    debug!("Updating node {} payload on blockchain", node_id);
                    let submitted = match node.set_payload(node_data) {
                        Ok(tx) => client.submit_finalized(&tx, &signer).await.map(|_| ()),
                        Err(e) => Err(e),
                    };
                    if let Err(e) = submitted {
                        // Blockchain update failed, log error and continue
                        error!("Failed to update node payload: {}", e);
                    } else {
                        trace!("Node {} payload updated successfully", node_id);
                    }
                }
                Ok(Event::Incoming(Packet::ConnAck(_))) => {
                    // Connected to MQTT broker
                }
                Ok(Event::Incoming(Packet::SubAck(_))) => {
                    // Subscription acknowledged
                }
                Ok(_) => {
                    // Other events, ignore
                }
                Err(_e) => {
                    // Connection error, wait and retry
                    sleep(Duration::from_secs(MQTT_RECONNECT_DELAY_SECS)).await;
                }
            }
        }
    }

    /// Monitor blockchain node and publish payload changes to MQTT topic.
    ///
    /// This method creates a long-running bridge that:
    /// 1. Connects to blockchain and MQTT broker
    /// 2. Subscribes to finalized blocks
    /// 3. Monitors for PayloadSet events for the specified node
    /// 4. Publishes node payload changes to the MQTT topic
    /// 5. Automatically handles MQTT reconnections
    ///
    /// # Arguments
    ///
    /// * `blockchain_config` - Configuration for blockchain connection
    /// * `topic` - MQTT topic to publish to
    /// * `node_id` - Blockchain node ID to monitor
    /// * `publish_handler` - Optional callback for publish notifications
    ///
    /// # Returns
    ///
    /// This function runs indefinitely and only returns on fatal errors.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # use mqtt_bridge::BlockchainConfig;
    /// # use mqtt_bridge as mqtt;
    /// # async fn example() -> anyhow::Result<()> {
    /// let blockchain_config = BlockchainConfig {
    ///     ws_url: "ws://localhost:9944".to_string(),
    ///     suri: Some("//Alice".to_string()),
    /// };
    ///
    /// let mqtt_config = mqtt::Config {
    ///     broker: "mqtt://localhost:1883".to_string(),
    ///     username: None,
    ///     password: None,
    ///     client_id: None,
    ///     blockchain: None,
    ///     subscribe: Vec::new(),
    ///     publish: Vec::new(),
    /// };
    ///
    /// mqtt_config.publish(
    ///     &blockchain_config,
    ///     None::<&libcps::crypto::Signer<libcps::crypto::Sr25519>>,  // Optional cipher for decryption
    ///     "actuators/status",
    ///     1,
    ///     None,
    /// ).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish<S: Scheme>(
        &self,
        blockchain_config: &BlockchainConfig,
        cipher: Option<&Signer<S>>,
        topic: &str,
        node_id: u64,
        publish_handler: Option<PublishHandler>,
    ) -> Result<()> {
        // Connect to blockchain
        let client = Client::connect(Some(&blockchain_config.ws_url)).await?;

        // Parse MQTT broker URL
        let (host, port) = parse_mqtt_url(&self.broker)?;

        // Configure MQTT client
        let client_id = self
            .client_id
            .clone()
            .unwrap_or_else(|| format!("cps-pub-{}", node_id));

        let mut mqttoptions = MqttOptions::new(client_id, host, port);
        mqttoptions.set_keep_alive(Duration::from_secs(30));

        // Set credentials if provided
        if let Some(username) = &self.username {
            mqttoptions.set_credentials(username, self.password.as_deref().unwrap_or(""));
        }

        // Create MQTT client
        let (mqtt_client, mut eventloop) = AsyncClient::new(mqttoptions, 10);

        // Create shutdown channel for graceful termination of background task
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

        // Spawn task to handle MQTT events (for auto-reconnect)
        let eventloop_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => {
                        // Shutdown signal received, exit gracefully
                        break;
                    }
                    result = eventloop.poll() => {
                        if let Err(_e) = result {
                            // Connection error, wait and retry
                            sleep(Duration::from_secs(MQTT_RECONNECT_DELAY_SECS)).await;
                        }
                    }
                }
            }
        });

        let node = NodeId::from(node_id);

        // Create node decrypt closure
        let node_data_to_string = |node_data: Vec<u8>| {
            // Try to decode as EncryptedMessage first
            if let Ok(message) = EncryptedMessage::decode(&mut node_data.as_slice()) {
                if let Some(cipher) = cipher {
                    let decrypted = cipher.decrypt(&message, None).map_err(|e| {
                        error!("Failed to decrypt message: {}", e);
                        anyhow!("Failed to decrypt message: {}", e)
                    })?;
                    String::from_utf8(decrypted).map_err(|e| {
                        error!("Invalid UTF-8 character: {}", e);
                        anyhow!("Invalid UTF-8 character: {}", e)
                    })
                } else {
                    serde_json::to_string(&message).map_err(|e| {
                        error!("Failed to convert encrypted message into JSON: {}", e);
                        anyhow!("Failed to convert encrypted message into JSON: {}", e)
                    })
                }
            } else {
                // Treat as plain data
                String::from_utf8(node_data).map_err(|e| {
                    error!("Invalid UTF-8 character: {}", e);
                    anyhow!("Invalid UTF-8 character: {}", e)
                })
            }
        };

        // Subscribe to payload updates of the node in finalized blocks
        let mut watcher = node
            .watch_payload(&client)
            .await
            .map_err(|e| anyhow!("Failed to subscribe to finalized blocks: {}", e))?;

        while let Some(update) = watcher.next().await {
            let update = match update {
                Ok(update) => update,
                Err(_e) => {
                    // Failed to process a block, skip
                    continue;
                }
            };

            if let Some(payload) = update.payload {
                // Extract or decrypt the data
                if let Ok(data) = node_data_to_string(payload) {
                    // Publish to MQTT
                    match mqtt_client
                        .publish(topic, QoS::AtMostOnce, false, data.as_bytes())
                        .await
                    {
                        Ok(_) => {
                            // Call publish handler if provided
                            if let Some(ref handler) = publish_handler {
                                handler(topic, update.block_number, &data);
                            }
                        }
                        Err(e) => {
                            error!("Failed to publish to MQTT: {}", e);
                        }
                    }
                }
            }
        }

        // Signal shutdown to the background MQTT event loop task
        let _ = shutdown_tx.send(());

        // Wait for the background task to finish (with timeout to avoid hanging)
        let _ = tokio::time::timeout(Duration::from_secs(5), eventloop_handle).await;

        Ok(())
    }
}

/// Constants for MQTT operation
const MQTT_RECONNECT_DELAY_SECS: u64 = 5;

/// Parse receiver public key from SS58 address or hex string.
///
/// Supports both SS58 addresses and hex-encoded 32-byte keys.
fn parse_receiver_public_key(addr_or_hex: &str) -> Result<[u8; 32]> {
    use libcps::AccountId;
    use std::str::FromStr;

    // Try SS58 decoding with AccountId (works for both Sr25519 and Ed25519)
    if let Ok(account_id) = AccountId::from_str(addr_or_hex) {
        return Ok(account_id.0);
    }

    // Fall back to hex decoding
    let hex_str = addr_or_hex.strip_prefix("0x").unwrap_or(addr_or_hex);
    let bytes = hex::decode(hex_str)
        .map_err(|e| anyhow!("Invalid receiver address (not valid SS58 or hex): {}", e))?;

    if bytes.len() != 32 {
        return Err(anyhow!(
            "Invalid receiver public key: expected 32 bytes, got {}",
            bytes.len()
        ));
    }

    let mut array = [0u8; 32];
    array.copy_from_slice(&bytes);
    Ok(array)
}

/// Parse MQTT broker URL to extract host and port.
///
/// Supports both `mqtt://` and `mqtts://` URL schemes.
/// Defaults to port 1883 if not specified.
///
/// # Examples
///
/// ```
/// # use mqtt_bridge::parse_mqtt_url;
/// let (host, port) = parse_mqtt_url("mqtt://localhost:1883").unwrap();
/// assert_eq!(host, "localhost");
/// assert_eq!(port, 1883);
///
/// let (host, port) = parse_mqtt_url("mqtt://broker.example.com").unwrap();
/// assert_eq!(host, "broker.example.com");
/// assert_eq!(port, 1883);
/// ```
pub fn parse_mqtt_url(url: &str) -> Result<(String, u16)> {
    let url = url.trim();

    // Remove mqtt:// or mqtts:// prefix if present
    let url = url
        .strip_prefix("mqtt://")
        .or_else(|| url.strip_prefix("mqtts://"))
        .unwrap_or(url);

    // Split host and port
    if let Some((host, port_str)) = url.split_once(':') {
        let port = port_str
            .parse::<u16>()
            .map_err(|_| anyhow!("Invalid port in MQTT URL: {}", port_str))?;
        Ok((host.to_string(), port))
    } else {
        // Default to port 1883 if not specified
        Ok((url.to_string(), 1883))
    }
}

/// Optional callback type for custom message handling in subscribe bridge.
///
/// When provided, this callback is called for each received MQTT message
/// before it's sent to the blockchain. Can be used for logging, validation,
/// or custom processing.
pub type MessageHandler = Box<dyn Fn(&str, &[u8]) + Send + Sync>;

/// Optional callback type for publish notifications.
///
/// When provided, this callback is called after successfully publishing
/// a message to MQTT. Can be used for logging or custom tracking.
///
/// Arguments: (topic, block_number, data)
pub type PublishHandler = Box<dyn Fn(&str, u64, &str) + Send + Sync>;

/// Transaction signer from the configured SURI.
fn signer(config: &BlockchainConfig) -> Result<Signer<Sr25519>> {
    let suri = config
        .suri
        .as_deref()
        .ok_or_else(|| anyhow!("This operation requires an account. Please provide a SURI."))?;
    Ok(Signer::from_suri(suri)?)
}

/// Cryptographic scheme of the encryption keypair, selected by name in the
/// command line and in configuration files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CryptoScheme {
    /// Schnorrkel SR25519 keys (Substrate native)
    Sr25519,
    /// ED25519 keys (IoT, Home Assistant)
    Ed25519,
}

impl FromStr for CryptoScheme {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "sr25519" | "sr" => Ok(Self::Sr25519),
            "ed25519" | "ed" => Ok(Self::Ed25519),
            _ => Err(anyhow!(
                "Invalid cryptographic scheme: '{s}'. Supported: sr25519, ed25519"
            )),
        }
    }
}

/// Run `$body` with `$scheme_ty` aliased to the `libcps` scheme marker type of a
/// [`CryptoScheme`], so generic code is instantiated for the scheme chosen at
/// runtime.
#[doc(hidden)]
#[macro_export]
macro_rules! with_scheme {
    ($scheme:expr, $scheme_ty:ident => $body:expr) => {
        match $scheme {
            $crate::CryptoScheme::Sr25519 => {
                type $scheme_ty = $crate::__libcps::crypto::Sr25519;
                $body
            }
            $crate::CryptoScheme::Ed25519 => {
                type $scheme_ty = $crate::__libcps::crypto::Ed25519;
                $body
            }
        }
    };
}
