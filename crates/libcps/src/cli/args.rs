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
//! Reusable `clap` argument groups shared by many commands.

use super::{config::WaitPolicy, output::OutputFormat, target::parse_account};
use clap::{Args, ValueEnum};
use libcps::{AccountId, Hash};

/// Connection and signer settings (global: accepted before or after the command).
#[derive(Args, Clone, Debug)]
pub struct ConnectionArgs {
    /// WebSocket URL of a node RPC endpoint [env: ROBONOMICS_WS_URL] [default:
    /// ws://127.0.0.1:9944, or the profile's `url`]. Run `cps light-client`
    /// (`cps lc`) for a local light client endpoint when no full node is available
    #[arg(long, global = true)]
    pub ws_url: Option<String>,

    /// Account secret URI (e.g. //Alice or a seed phrase). Signs transactions
    /// (SR25519) and is the key for encryption and decryption
    #[arg(long, global = true, env = "ROBONOMICS_SURI", hide_env_values = true)]
    pub suri: Option<String>,
}

/// Result format and transaction wait policy (global).
#[derive(Args, Clone, Debug)]
pub struct OutputArgs {
    /// Output format: `human` text or `json` (watches emit JSON Lines)
    /// [env: CPS_OUTPUT] [default: human, or the profile's `output`]
    #[arg(long, global = true, value_enum)]
    pub output: Option<OutputFormat>,

    /// How long to wait for a transaction: `best` (included in a best block) or
    /// `finalized` [env: CPS_WAIT] [default: finalized, or the profile's `wait`]
    #[arg(long, global = true, value_enum)]
    pub wait: Option<WaitPolicy>,
}

/// Configuration file and profile selection (global).
#[derive(Args, Clone, Debug)]
pub struct ConfigArgs {
    /// Configuration file (TOML) [env: CPS_CONFIG] [default:
    /// $XDG_CONFIG_HOME/cps/config.toml]
    #[arg(long, global = true, value_name = "FILE")]
    pub config: Option<std::path::PathBuf>,

    /// Profile of the configuration file [env: CPS_PROFILE] [default: the
    /// file's `default-profile`]
    #[arg(long, global = true, value_name = "NAME")]
    pub profile: Option<String>,
}

/// Pinned-read selection; read commands only, writes never accept it.
#[derive(Args, Clone, Debug, Default)]
pub struct ReadArgs {
    /// Read the state at this block hash instead of the latest finalized block
    #[arg(long, value_name = "BLOCK_HASH", value_parser = parse_hash)]
    pub at: Option<Hash>,
}

/// Parse a `0x`-prefixed or bare 32-byte hex block hash.
pub fn parse_hash(text: &str) -> Result<Hash, String> {
    let bytes = hex::decode(text.strip_prefix("0x").unwrap_or(text))
        .map_err(|_| format!("`{text}` is not a hex hash"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|b: Vec<u8>| format!("block hash must be 32 bytes, got {}", b.len()))?;
    Ok(Hash::from(bytes))
}

/// Account argument: SS58 address or `0x` hex public key.
pub fn account_arg(text: &str) -> Result<AccountId, String> {
    parse_account(text)
}

/// Encryption scheme of the SURI keypair (sr25519 and ed25519 support ECDH).
#[derive(Clone, Copy, Debug, Default, ValueEnum)]
pub enum CipherScheme {
    /// Schnorrkel SR25519 keys (Substrate native)
    #[default]
    #[value(alias = "sr")]
    Sr25519,
    /// ED25519 keys (IoT, Home Assistant)
    #[value(alias = "ed")]
    Ed25519,
}

/// Encrypt written data for a receiver.
#[derive(Args, Clone, Debug)]
pub struct EncryptionArgs {
    /// Encrypt the data for this receiver (SS58 address or hex public key); needs --suri
    #[arg(short = 'r', long)]
    pub receiver_public: Option<String>,

    /// Cipher: xchacha20, aesgcm256 or chacha20
    #[arg(long, default_value = "xchacha20")]
    pub cipher: String,

    /// Scheme of the --suri keypair used for the key agreement
    #[arg(long, value_enum, ignore_case = true, default_value_t = CipherScheme::Sr25519)]
    pub scheme: CipherScheme,
}

/// Decrypt read data.
#[derive(Args, Clone, Debug)]
pub struct DecryptArgs {
    /// Decrypt encrypted data with the --suri keypair
    #[arg(short = 'd', long)]
    pub decrypt: bool,

    /// Scheme of the --suri keypair used for the key agreement
    #[arg(long, value_enum, ignore_case = true, default_value_t = CipherScheme::Sr25519)]
    pub scheme: CipherScheme,
}

/// Data to store, as bytes: an argument, a file or standard input.
#[derive(Args, Clone, Debug)]
pub struct DataInputArgs {
    /// Data as text; `-` reads standard input
    #[arg(
        value_name = "DATA",
        conflicts_with = "file",
        required_unless_present = "file"
    )]
    pub data: Option<String>,

    /// Read the data (binary allowed) from a file
    #[arg(long, value_name = "FILE")]
    pub file: Option<std::path::PathBuf>,
}

/// Identity permit artifact input.
#[derive(Args, Clone, Debug)]
pub struct PermitInputArgs {
    /// Identity permit artifact (JSON) from a file, or `-` for standard input.
    /// The identity is taken from the permit
    #[arg(long, value_name = "FILE|-")]
    pub permit: Option<String>,
}
