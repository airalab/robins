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
//! # libcps - Robonomics Cyber-Physical Systems Library
//!
//! `libcps` provides a comprehensive Rust library for interacting with the Robonomics
//! CPS (Cyber-Physical Systems) pallet. It enables developers to build applications
//! that manage hierarchical cyber-physical systems on the Robonomics blockchain with
//! support for encrypted data storage and IoT integration.
//!
//! ## Features
//!
//! - **Blockchain Integration**: Seamless interaction with Robonomics blockchain via subxt
//! - **Encryption**: XChaCha20-Poly1305 AEAD encryption with sr25519 key derivation
//! - **Type Safety**: Strongly-typed APIs matching the CPS pallet
//! - **Async Support**: Built on tokio for efficient async operations
//!
//! ## Quick Start
//!
//! ```no_run
//! use libcps::blockchain::{Client, Config};
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Connect to blockchain
//!     let config = Config {
//!         ws_url: "ws://localhost:9944".to_string(),
//!         suri: Some("//Alice".to_string()),
//!     };
//!     
//!     let client = Client::new(&config).await?;
//!     
//!     // Use the client to interact with CPS pallet
//!     // (metadata is auto-generated from runtime dependency)
//!     
//!     Ok(())
//! }
//! ```
//!
//! ## Modules
//!
//! - [`blockchain`]: Blockchain client and connection management
//! - [`crypto`]: Encryption and key derivation utilities
//! - [`node`]: Node-oriented API with type definitions and async methods for CPS operations
//!
//! ## Encryption
//!
//! The library implements **AEAD encryption with multiple algorithms and schemes**:
//!
//! ```no_run
//! use libcps::crypto::{Cipher, EncryptionAlgorithm, CryptoScheme};
//!
//! # fn example() -> anyhow::Result<()> {
//! // Create a Cipher with SR25519 scheme
//! let sender_cipher = Cipher::new(
//!     "//Alice".to_string(),
//!     CryptoScheme::Sr25519,
//! )?;
//!
//! let receiver_cipher = Cipher::new(
//!     "//Bob".to_string(),
//!     CryptoScheme::Sr25519,
//! )?;
//!
//! let plaintext = b"secret message";
//! let receiver_public = receiver_cipher.public_key();
//!
//! // Encrypt using the cipher
//! let encrypted_msg = sender_cipher.encrypt(plaintext, &receiver_public, EncryptionAlgorithm::XChaCha20Poly1305)?;
//!
//! // Decrypt with optional sender verification
//! let sender_public = sender_cipher.public_key();
//! let decrypted = receiver_cipher.decrypt(&encrypted_msg, Some(&sender_public))?;
//! # Ok(())
//! # }
//! ```
//!
//! ## MQTT Bridge
//!
//! MQTT/IoT integration has moved to the dedicated `mqtt-bridge` crate, which
//! depends on this library for blockchain, crypto, and node operations. See the
//! `mqtt-bridge` crate documentation and its `examples/mqtt_config.toml` for
//! configuration and usage details.
//!
//! ## Feature Flags
//!
//! The library supports optional features:
//!
//! - **`cli`** (default) - Enables CLI binary with colored output
//!
//! ```toml
//! # Default (CLI enabled)
//! libcps = "0.1.0"
//!
//! # Library only, no CLI
//! libcps = { version = "0.1.0", default-features = false }
//! ```
//!
//! ## Type Definitions
//!
//! The library provides types that match the CPS pallet:
//!
//! ```
//! use libcps::blockchain::BoundedVec;
//! use libcps::node::NodeId;
//!
//! let node_id = NodeId(42);
//! let plain_data = BoundedVec(b"sensor reading".to_vec());
//! let encrypted_data = BoundedVec(vec![1, 2, 3, 4]);
//! ```
//!
//! ## Crates.io Metadata
//!
//! - **Repository**: <https://github.com/airalab/robonomics>
//! - **Documentation**: <https://docs.rs/libcps>
//! - **License**: Apache-2.0
//!
//! ## Safety
//!
//! This crate uses `#![forbid(unsafe_code)]` to ensure memory safety.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod blockchain;
pub mod crypto;
pub mod node;
