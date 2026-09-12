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
//! CPS CLI - Command-line interface for Robonomics CPS pallet.
//!
//! This binary provides a beautiful, user-friendly CLI for managing
//! cyber-physical systems on the Robonomics blockchain.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::str::FromStr;
use subxt::utils::AccountId32;

// Import from the library
use libcps::blockchain;
use libcps::crypto::{Cipher, EncryptionAlgorithm};

// CLI-specific modules (display and commands)
mod commands;
mod display;

/// Parses a receiver public key from either an SS58 address or a hex-encoded 32-byte key.
///
/// # Supported formats
/// - **SS58 address**: A valid Substrate SS58-encoded account ID. Decoding is attempted first
///   using subxt's `AccountId32::from_str`, which supports both Sr25519 and
///   Ed25519 (they share the same 32-byte public key length).
/// - **Hex string**: A 64-hex-character string representing a 32-byte public key. An optional
///   `0x` prefix is allowed (e.g. `0xdeadbeef...` or `deadbeef...`).
///
/// # Conversion process
/// 1. Try to decode `addr_or_hex` as an SS58 address. On success, the underlying 32-byte
///    account ID is returned.
/// 2. If SS58 decoding fails, strip a leading `0x` (if present) and attempt to decode the
///    remaining string as hex.
///
/// # Errors
/// - Returns an error if the value is neither a valid SS58 address nor a valid hex string.
/// - Returns an error if the hex decoding succeeds but the resulting byte length is not
///   exactly 32 bytes.
fn parse_receiver_public_key(addr_or_hex: &str) -> Result<[u8; 32]> {
    // Try SS58 decoding with AccountId32 (works for both Sr25519 and Ed25519)
    if let Ok(account_id) = AccountId32::from_str(addr_or_hex) {
        return Ok(account_id.0);
    }

    // Fall back to hex decoding
    let hex_str = addr_or_hex.strip_prefix("0x").unwrap_or(addr_or_hex);
    let bytes = hex::decode(hex_str)
        .map_err(|e| anyhow::anyhow!("Invalid receiver address (not valid SS58 or hex): {}", e))?;

    if bytes.len() != 32 {
        return Err(anyhow::anyhow!(
            "Invalid receiver public key: expected 32 bytes, got {}",
            bytes.len()
        ));
    }

    let mut array = [0u8; 32];
    array.copy_from_slice(&bytes);
    Ok(array)
}

#[derive(Parser)]
#[command(name = "cps")]
#[command(version, about = "libcps - Robonomics Cyber-Physical System controls", long_about = None)]
#[command(before_help = r#"
╔══════════════════════════════════════════════════════╗
║                                                      ║
║     ██╗     ██╗██████╗  ██████╗██████╗ ███████╗      ║
║     ██║     ██║██╔══██╗██╔════╝██╔══██╗██╔════╝      ║
║     ██║     ██║██████╔╝██║     ██████╔╝███████╗      ║
║     ██║     ██║██╔══██╗██║     ██╔═══╝ ╚════██║      ║
║     ███████╗██║██████╔╝╚██████╗██║     ███████║      ║
║     ╚══════╝╚═╝╚═════╝  ╚═════╝╚═╝     ╚══════╝      ║
║                                                      ║
║     Cyber-Physical Systems - Robonomics Network      ║
║                                                      ║
╚══════════════════════════════════════════════════════╝
"#)]
struct Cli {
    /// WebSocket URL for blockchain connection
    #[arg(long, env = "ROBONOMICS_WS_URL", default_value = "ws://localhost:9944")]
    ws_url: String,

    /// Account secret URI (e.g., //Alice, //Bob, or seed phrase)
    #[arg(long, env = "ROBONOMICS_SURI")]
    suri: Option<String>,

    /// Logging level (off, error, warn, info, debug, trace)
    #[arg(short = 'l', long, env = "RUST_LOG", default_value = "warn")]
    log_level: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Display node information and its children in a beautiful tree format
    #[command(
        long_about = "Display node information and its children in a tree format.

EXAMPLES:
    # Show node 0
    cps show 0

    # Show node with decryption attempt (using SR25519)
    cps show 5 --decrypt

    # Show node with ED25519 decryption
    cps show 5 --decrypt --scheme ed25519"
    )]
    Show {
        /// Node ID to display
        node_id: u64,

        /// Attempt to decrypt encrypted data
        #[arg(short = 'd', long)]
        decrypt: bool,

        /// Cryptographic scheme for decryption (sr25519, ed25519)
        #[arg(long, default_value = "sr25519", value_parser = clap::value_parser!(libcps::crypto::CryptoScheme))]
        scheme: libcps::crypto::CryptoScheme,
    },

    /// Create a new node (root or child)
    #[command(long_about = "Create a new node (root or child).

EXAMPLES:
    # Create root node
    cps create --meta '{\"type\":\"sensor\"}' --payload '22.5C'

    # Create child node
    cps create --parent 0 --payload 'operational data'

    # Create with encryption (SR25519, default)
    cps create --parent 0 --payload 'secret data' \\
        --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY

    # Create with ED25519 encryption (Home Assistant compatible)
    cps create --parent 0 --payload 'secret data' \\
        --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY \\
        --scheme ed25519

    # Create with specific cipher
    cps create --parent 0 --payload 'secret data' \\
        --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY \\
        --cipher aesgcm256")]
    Create {
        /// Parent node ID (omit for root node)
        #[arg(short = 'p', long)]
        parent: Option<u64>,

        /// Metadata (configuration data)
        #[arg(long)]
        meta: Option<String>,

        /// Payload (operational data)
        #[arg(long)]
        payload: Option<String>,

        /// Receiver public key or SS58 address for encryption. If provided, data will be encrypted.
        /// Supports both SS58 addresses and hex-encoded public keys.
        #[arg(short = 'r', long)]
        receiver_public: Option<String>,

        /// Encryption algorithm (xchacha20, aesgcm256, chacha20)
        #[arg(long, default_value = "xchacha20")]
        cipher: String,

        /// Cryptographic scheme for encryption (sr25519, ed25519)
        #[arg(long, default_value = "sr25519", value_parser = clap::value_parser!(libcps::crypto::CryptoScheme))]
        scheme: libcps::crypto::CryptoScheme,
    },

    /// Update node metadata
    #[command(long_about = "Update node metadata.

EXAMPLES:
    # Update metadata
    cps set-meta 5 '{\"name\":\"Updated Sensor\"}'

    # Update with encryption
    cps set-meta 5 'private config' \\
        --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY

    # Update with ED25519 encryption
    cps set-meta 5 'private config' \\
        --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY \\
        --scheme ed25519")]
    SetMeta {
        /// Node ID
        node_id: u64,

        /// New metadata
        data: String,

        /// Receiver public key or SS58 address for encryption. If provided, data will be encrypted.
        /// Supports both SS58 addresses and hex-encoded public keys.
        #[arg(short = 'r', long)]
        receiver_public: Option<String>,

        /// Encryption algorithm (xchacha20, aesgcm256, chacha20)
        #[arg(long, default_value = "xchacha20")]
        cipher: String,

        /// Cryptographic scheme for encryption (sr25519, ed25519)
        #[arg(long, default_value = "sr25519", value_parser = clap::value_parser!(libcps::crypto::CryptoScheme))]
        scheme: libcps::crypto::CryptoScheme,
    },

    /// Update node payload
    #[command(long_about = "Update node payload (operational data).

EXAMPLES:
    # Update temperature reading
    cps set-payload 5 '23.1C'

    # Update with encryption
    cps set-payload 5 'encrypted telemetry' \\
        --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY

    # Update with ED25519 and AES-GCM
    cps set-payload 5 'encrypted telemetry' \\
        --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY \\
        --scheme ed25519 --cipher aesgcm256")]
    SetPayload {
        /// Node ID
        node_id: u64,

        /// New payload
        data: String,

        /// Receiver public key or SS58 address for encryption. If provided, data will be encrypted.
        /// Supports both SS58 addresses and hex-encoded public keys.
        #[arg(short = 'r', long)]
        receiver_public: Option<String>,

        /// Encryption algorithm (xchacha20, aesgcm256, chacha20)
        #[arg(long, default_value = "xchacha20")]
        cipher: String,

        /// Cryptographic scheme for encryption (sr25519, ed25519)
        #[arg(long, default_value = "sr25519", value_parser = clap::value_parser!(libcps::crypto::CryptoScheme))]
        scheme: libcps::crypto::CryptoScheme,
    },

    /// Move a node to a new parent
    #[command(long_about = "Move a node to a new parent.

EXAMPLES:
    # Move node 5 under node 3
    cps move 5 3

FEATURES:
    - Automatic cycle detection (prevents moving a node under its own descendant)
    - Path validation")]
    Move {
        /// Node ID to move
        node_id: u64,

        /// New parent node ID
        new_parent_id: u64,
    },

    /// Delete a node (must have no children)
    #[command(long_about = "Delete a node (must have no children).

EXAMPLES:
    # Remove node with confirmation
    cps remove 5

    # Remove without confirmation
    cps remove 5 --force")]
    Remove {
        /// Node ID to remove
        node_id: u64,

        /// Skip confirmation prompt
        #[arg(short = 'f', long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialize logging
    std::env::set_var("RUST_LOG", &cli.log_level);
    env_logger::init();

    // Create blockchain config (crypto-free)
    let blockchain_config = blockchain::Config {
        ws_url: cli.ws_url.clone(),
        suri: cli.suri.clone(),
    };

    // Execute commands
    match cli.command {
        Commands::Show {
            node_id,
            decrypt,
            scheme,
        } => {
            // Create cipher if decryption is requested
            let cipher = if decrypt {
                let suri = cli
                    .suri
                    .ok_or_else(|| anyhow::anyhow!("SURI required for decryption"))?;
                Some(Cipher::new(suri, scheme)?)
            } else {
                None
            };
            commands::show::execute(&blockchain_config, cipher.as_ref(), node_id).await?;
        }
        Commands::Create {
            parent,
            meta,
            payload,
            receiver_public,
            cipher,
            scheme,
        } => {
            // Parse receiver public key if provided (supports both SS58 address and hex)
            let receiver_pub_bytes = if let Some(ref addr_or_hex) = receiver_public {
                Some(parse_receiver_public_key(addr_or_hex)?)
            } else {
                None
            };

            // Encryption requires BOTH sender SURI and receiver public key.
            // - SURI (sender's seed phrase): Used to derive the sender's keypair for ECDH
            // - receiver_public: The recipient's public key for deriving the shared secret
            // If receiver_public is None, data will be stored as plaintext (no encryption).
            let (cipher_opt, algorithm_opt) = if receiver_public.is_some() {
                let algorithm = libcps::crypto::EncryptionAlgorithm::from_str(&cipher)
                    .map_err(|e| anyhow::anyhow!("Invalid cipher: {}", e))?;
                let suri = cli
                    .suri
                    .ok_or_else(|| anyhow::anyhow!("SURI required for encryption"))?;
                (Some(Cipher::new(suri, scheme)?), Some(algorithm))
            } else {
                (None, None)
            };
            commands::create::execute(
                &blockchain_config,
                cipher_opt.as_ref(),
                parent,
                meta,
                payload,
                receiver_pub_bytes,
                algorithm_opt,
            )
            .await?;
        }
        Commands::SetMeta {
            node_id,
            data,
            receiver_public,
            cipher,
            scheme,
        } => {
            // Parse receiver public key if provided (supports both SS58 address and hex)
            let receiver_pub_bytes = if let Some(ref addr_or_hex) = receiver_public {
                Some(parse_receiver_public_key(addr_or_hex)?)
            } else {
                None
            };

            // Create cipher if encryption is requested
            let (cipher_opt, algorithm_opt) = if receiver_public.is_some() {
                let algorithm = EncryptionAlgorithm::from_str(&cipher)
                    .map_err(|e| anyhow::anyhow!("Invalid cipher: {}", e))?;
                let suri = cli
                    .suri
                    .ok_or_else(|| anyhow::anyhow!("SURI required for encryption"))?;
                (Some(Cipher::new(suri, scheme)?), Some(algorithm))
            } else {
                (None, None)
            };
            commands::set_meta::execute(
                &blockchain_config,
                cipher_opt.as_ref(),
                node_id,
                data,
                receiver_pub_bytes,
                algorithm_opt,
            )
            .await?;
        }
        Commands::SetPayload {
            node_id,
            data,
            receiver_public,
            cipher,
            scheme,
        } => {
            // Parse receiver public key if provided (supports both SS58 address and hex)
            let receiver_pub_bytes = if let Some(ref addr_or_hex) = receiver_public {
                Some(parse_receiver_public_key(addr_or_hex)?)
            } else {
                None
            };

            // Create cipher if encryption is requested
            let (cipher_opt, algorithm_opt) = if receiver_public.is_some() {
                let algorithm = EncryptionAlgorithm::from_str(&cipher)
                    .map_err(|e| anyhow::anyhow!("Invalid cipher: {}", e))?;
                let suri = cli
                    .suri
                    .ok_or_else(|| anyhow::anyhow!("SURI required for encryption"))?;
                (Some(Cipher::new(suri, scheme)?), Some(algorithm))
            } else {
                (None, None)
            };
            commands::set_payload::execute(
                &blockchain_config,
                cipher_opt.as_ref(),
                node_id,
                data,
                receiver_pub_bytes,
                algorithm_opt,
            )
            .await?;
        }
        Commands::Move {
            node_id,
            new_parent_id,
        } => {
            commands::move_node::execute(&blockchain_config, node_id, new_parent_id).await?;
        }
        Commands::Remove { node_id, force } => {
            commands::remove::execute(&blockchain_config, node_id, force).await?;
        }
    }

    Ok(())
}
