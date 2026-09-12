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
//! `edge key` — sensor identity management: generate, inspect, sign and verify.
//!
//! A sensor identity is an Ed25519 keypair whose public key *is* the
//! [`SensorId`](crate::protocol::SensorId) (an `AccountId32` rendered as an
//! SS58 address). This mirrors Substrate's `subkey`: `generate` mints a fresh
//! key and `inspect` reports the public material for an existing one, both in
//! the familiar "Secret seed / Public key / SS58 Address" layout.
//!
//! Signing and verification reuse the canonical [`crate::protocol`] routines and
//! the shared stdin/stdout plumbing from [`super::codec`]; no crypto is
//! re-implemented here.

use super::codec::{
    load_identity, protocol_err, read_envelope_bytes, read_stdin, wire_from_input, write_repr,
    ByteFormat, ReportFormat, ReprFormat,
};
use super::{CliError, CliResult};
use crate::protocol::{self, SensorId, SensorIdentity, SignOptions};
use clap::Subcommand;
use std::path::PathBuf;

/// `edge key` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum KeyCommand {
    /// Generate a new random Ed25519 sensor identity.
    Generate {
        /// Report rendering.
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        output: ReportFormat,
    },
    /// Inspect a Key URI, reporting its public identity (subkey-style).
    Inspect {
        /// A Key URI to be inspected: an SS58 address (public-only) or a
        /// `0x`-prefixed / bare 32-byte hex secret seed.
        uri: String,
        /// Report rendering.
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        output: ReportFormat,
    },
    /// Sign an opaque message, producing a `SignedEnvelope`.
    Sign {
        /// Path to a file holding the Ed25519 secret key (32 raw bytes or hex).
        #[arg(long)]
        key: PathBuf,
        /// Measurement timestamp in Unix milliseconds (defaults to now).
        #[arg(long)]
        timestamp: Option<u64>,
        /// Anti-replay nonce as hex (defaults to a fresh random value).
        #[arg(long)]
        nonce: Option<String>,
        /// Format of the message read from stdin.
        #[arg(long, value_enum, default_value_t = ByteFormat::Binary)]
        input: ByteFormat,
        /// Format of the envelope written to stdout.
        #[arg(long, value_enum, default_value_t = ReprFormat::Binary)]
        output: ReprFormat,
    },
    /// Verify an envelope's signature and structure.
    Verify {
        /// Format of the envelope read from stdin.
        #[arg(long, value_enum, default_value_t = ByteFormat::Binary)]
        input: ByteFormat,
        /// Report rendering.
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        output: ReportFormat,
    },
}

/// Dispatch a `key` subcommand.
pub(crate) fn run(command: KeyCommand) -> CliResult {
    match command {
        KeyCommand::Generate { output } => generate(output),
        KeyCommand::Inspect { uri, output } => inspect(uri, output),
        KeyCommand::Sign {
            key,
            timestamp,
            nonce,
            input,
            output,
        } => sign(key, timestamp, nonce, input, output),
        KeyCommand::Verify { input, output } => verify(input, output),
    }
}

/// Implements `edge key generate`.
fn generate(output: ReportFormat) -> CliResult {
    let identity = SensorIdentity::generate();
    report(
        Some(identity.secret_to_hex()),
        &identity.sensor_id(),
        output,
    )
}

/// Implements `edge key inspect`.
///
/// Following `subkey`, the URI may be a public SS58 address (reported without a
/// secret) or a 32-byte hex secret seed (reported with its derived public key).
fn inspect(uri: String, output: ReportFormat) -> CliResult {
    let uri = uri.trim();

    // An SS58 address yields a public-only report (no secret is recoverable).
    if let Ok(sensor_id) = SensorId::from_ss58(uri) {
        return report(None, &sensor_id, output);
    }

    // Otherwise interpret the URI as a hex secret seed (`0x`-prefix optional).
    let seed = uri.strip_prefix("0x").unwrap_or(uri);
    let identity = SensorIdentity::from_hex(seed).map_err(|e| {
        CliError::usage(format!(
            "invalid key URI: expected an SS58 address or 32-byte hex secret seed: {e}"
        ))
    })?;
    report(
        Some(identity.secret_to_hex()),
        &identity.sensor_id(),
        output,
    )
}

/// Render a sensor identity in the `subkey`-style key report.
///
/// The `secret_hex` is only present when the caller holds the private key (i.e.
/// not when inspecting a public SS58 address).
fn report(secret_hex: Option<String>, sensor_id: &SensorId, output: ReportFormat) -> CliResult {
    let public_hex = sensor_id.to_hex();
    let ss58 = sensor_id.to_ss58();

    match output {
        ReportFormat::Text => {
            if let Some(secret) = &secret_hex {
                println!("Secret seed:  0x{secret}");
            }
            println!("Public key:   0x{public_hex}");
            println!("SS58 Address: {ss58}");
        }
        ReportFormat::Json => {
            let mut map = serde_json::Map::new();
            if let Some(secret) = &secret_hex {
                map.insert(
                    "secretSeed".into(),
                    serde_json::json!(format!("0x{secret}")),
                );
            }
            map.insert(
                "publicKey".into(),
                serde_json::json!(format!("0x{public_hex}")),
            );
            map.insert(
                "sensorId".into(),
                serde_json::json!(format!("0x{public_hex}")),
            );
            map.insert("ss58Address".into(), serde_json::json!(ss58));
            let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))
                .map_err(|e| CliError::runtime(format!("failed to serialize JSON: {e}")))?;
            println!("{json}");
        }
    }
    Ok(())
}

/// Implements `edge key sign`.
fn sign(
    key: PathBuf,
    timestamp: Option<u64>,
    nonce: Option<String>,
    input: ByteFormat,
    output: ReprFormat,
) -> CliResult {
    let identity = load_identity(&key)?;
    let message = wire_from_input(&read_stdin()?, input)?;
    let nonce = match nonce {
        Some(hex_nonce) => Some(
            hex::decode(hex_nonce.trim())
                .map_err(|e| CliError::usage(format!("invalid --nonce hex: {e}")))?,
        ),
        None => None,
    };
    let options = SignOptions {
        timestamp_ms: timestamp,
        nonce,
    };
    let env = protocol::sign_message(&identity, &message, options);
    let wire = protocol::encode_envelope(&env);
    write_repr(&env, &wire, output)
}

/// Implements `edge key verify`.
fn verify(input: ByteFormat, output: ReportFormat) -> CliResult {
    let (env, _wire) = read_envelope_bytes(input)?;
    match protocol::verify_envelope(&env) {
        Ok(verified) => {
            match output {
                ReportFormat::Text => println!("valid"),
                ReportFormat::Json => {
                    let value = serde_json::json!({
                        "valid": true,
                        "sensor_id": verified.sensor_id.to_ss58(),
                    });
                    println!("{value}");
                }
            }
            Ok(())
        }
        Err(err) => {
            if output == ReportFormat::Json {
                let value = serde_json::json!({
                    "valid": false,
                    "error": err.to_string(),
                });
                println!("{value}");
            }
            Err(protocol_err(err))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_identity_signs_and_verifies() {
        let identity = SensorIdentity::generate();
        let env = protocol::sign_message(&identity, b"payload", SignOptions::default());
        let verified = protocol::verify_envelope(&env).unwrap();
        assert_eq!(verified.sensor_id, identity.sensor_id());
    }

    #[test]
    fn inspect_reports_matching_ss58() {
        let identity = SensorIdentity::from_secret_bytes(&[7u8; 32]);
        // The rendered SS58 must round-trip back to the same account bytes.
        let ss58 = identity.sensor_id().to_ss58();
        let parsed = crate::protocol::SensorId::from_ss58(&ss58).unwrap();
        assert_eq!(parsed, identity.sensor_id());
    }
}
