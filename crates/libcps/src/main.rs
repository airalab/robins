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
use clap::{Parser, Subcommand, ValueEnum};
use std::str::FromStr;

// Import from the library
use libcps::crypto::{EncryptionAlgorithm, Scheme, Signer};
use libcps::AccountId;

// CLI-specific modules (display and commands)
mod commands;
mod display;

/// Cryptographic scheme of the encryption keypair (CLI selection only).
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum SchemeArg {
    /// Schnorrkel SR25519 keys (Substrate native)
    #[default]
    #[value(alias = "sr")]
    Sr25519,
    /// ED25519 keys (IoT, Home Assistant)
    #[value(alias = "ed")]
    Ed25519,
}

/// Run `$body` with `$keypair` aliased to the scheme marker type of a [`SchemeArg`], so
/// generic code is instantiated for the scheme chosen at runtime.
macro_rules! with_scheme {
    ($scheme:expr, $keypair:ident => $body:expr) => {
        match $scheme {
            SchemeArg::Sr25519 => {
                type $keypair = libcps::crypto::Sr25519;
                $body
            }
            SchemeArg::Ed25519 => {
                type $keypair = libcps::crypto::Ed25519;
                $body
            }
        }
    };
}

/// Parses a receiver public key from either an SS58 address or a hex-encoded 32-byte key.
///
/// # Supported formats
/// - **SS58 address**: A valid Substrate SS58-encoded account ID. Decoding is attempted first
///   using `AccountId::from_str`, which works for both SR25519 and ED25519 accounts
///   (they share the same 32-byte public key length).
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
    // Try SS58 decoding with AccountId (works for both Sr25519 and Ed25519)
    if let Ok(account_id) = AccountId::from_str(addr_or_hex) {
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
#[command(version, about = "Robonomics Cyber-Physical System toolbox", long_about = None)]
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
║     Cyber-Physical System - Robonomics Network       ║
║                                                      ║
╚══════════════════════════════════════════════════════╝
"#)]
struct Cli {
    /// WebSocket URL of a node RPC endpoint; an embedded light client is used when omitted
    #[arg(long, env = "ROBONOMICS_WS_URL")]
    ws_url: Option<String>,

    /// Account secret URI (e.g., //Alice, //Bob, or seed phrase). Signs transactions
    /// (SR25519) and is the key for encryption and decryption
    #[arg(long, env = "ROBONOMICS_SURI")]
    suri: Option<String>,

    /// Log filter: a level (off, error, warn, info, debug, trace) or tracing
    /// directives such as `info,subxt=warn`
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
        #[arg(long, value_enum, ignore_case = true, default_value_t = SchemeArg::Sr25519)]
        scheme: SchemeArg,
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
        #[arg(long, value_enum, ignore_case = true, default_value_t = SchemeArg::Sr25519)]
        scheme: SchemeArg,
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
        #[arg(long, value_enum, ignore_case = true, default_value_t = SchemeArg::Sr25519)]
        scheme: SchemeArg,
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
        #[arg(long, value_enum, ignore_case = true, default_value_t = SchemeArg::Sr25519)]
        scheme: SchemeArg,
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(&cli.log_level)
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    let connection = commands::Connection {
        url: cli.ws_url.clone(),
        suri: cli.suri.clone(),
    };

    // Execute commands
    match cli.command {
        Commands::Show {
            node_id,
            decrypt,
            scheme,
        } => {
            with_scheme!(scheme, P => {
                // Create keypair if decryption is requested
                let cipher = cipher_keypair::<P>(&cli.suri, decrypt, "decryption")?;
                commands::show::execute(&connection, cipher.as_ref(), node_id).await?
            });
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
            let receiver_pub_bytes = receiver_public
                .as_deref()
                .map(parse_receiver_public_key)
                .transpose()?;

            // Encryption requires BOTH sender SURI and receiver public key.
            // - SURI (sender's seed phrase): Used to derive the sender's keypair for ECDH
            // - receiver_public: The recipient's public key for deriving the shared secret
            // If receiver_public is None, data will be stored as plaintext (no encryption).
            let algorithm_opt = parse_algorithm(receiver_public.is_some(), &cipher)?;
            with_scheme!(scheme, P => {
                let keypair =
                    cipher_keypair::<P>(&cli.suri, receiver_public.is_some(), "encryption")?;
                commands::create::execute(
                    &connection,
                    keypair.as_ref(),
                    parent,
                    meta,
                    payload,
                    receiver_pub_bytes,
                    algorithm_opt,
                )
                .await?
            });
        }
        Commands::SetMeta {
            node_id,
            data,
            receiver_public,
            cipher,
            scheme,
        } => {
            let receiver_pub_bytes = receiver_public
                .as_deref()
                .map(parse_receiver_public_key)
                .transpose()?;
            let algorithm_opt = parse_algorithm(receiver_public.is_some(), &cipher)?;
            with_scheme!(scheme, P => {
                let keypair =
                    cipher_keypair::<P>(&cli.suri, receiver_public.is_some(), "encryption")?;
                commands::set_meta::execute(
                    &connection,
                    keypair.as_ref(),
                    node_id,
                    data,
                    receiver_pub_bytes,
                    algorithm_opt,
                )
                .await?
            });
        }
        Commands::SetPayload {
            node_id,
            data,
            receiver_public,
            cipher,
            scheme,
        } => {
            let receiver_pub_bytes = receiver_public
                .as_deref()
                .map(parse_receiver_public_key)
                .transpose()?;
            let algorithm_opt = parse_algorithm(receiver_public.is_some(), &cipher)?;
            with_scheme!(scheme, P => {
                let keypair =
                    cipher_keypair::<P>(&cli.suri, receiver_public.is_some(), "encryption")?;
                commands::set_payload::execute(
                    &connection,
                    keypair.as_ref(),
                    node_id,
                    data,
                    receiver_pub_bytes,
                    algorithm_opt,
                )
                .await?
            });
        }
        Commands::Remove { node_id, force } => {
            commands::remove::execute(&connection, node_id, force).await?;
        }
    }

    Ok(())
}

/// Signer for encryption or decryption of the scheme `S`, created only when
/// `needed`.
fn cipher_keypair<S: Scheme>(
    suri: &Option<String>,
    needed: bool,
    purpose: &str,
) -> Result<Option<Signer<S>>> {
    if !needed {
        return Ok(None);
    }
    let suri = suri
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("SURI required for {purpose}"))?;
    Ok(Some(Signer::from_suri(suri)?))
}

/// Parse the `--cipher` name when encryption is requested.
fn parse_algorithm(encrypting: bool, name: &str) -> Result<Option<EncryptionAlgorithm>> {
    if !encrypting {
        return Ok(None);
    }
    EncryptionAlgorithm::from_str(name)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("Invalid cipher: {}", e))
}
