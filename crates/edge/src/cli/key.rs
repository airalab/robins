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
//! `edge key` — sensor identity management: generate and inspect.
//!
//! A sensor identity is an Ed25519 keypair whose public key *is* the
//! [`SensorId`](crate::protocol::SensorId) (an `AccountId32` rendered as an
//! SS58 address). This mirrors Substrate's `subkey`: `generate` mints a fresh
//! key and `inspect` reports the public material for an existing one, both in
//! the familiar "Secret seed / Public key / SS58 Address" layout.
//!
//! Signing and verification of envelopes live under `edge envelope`; this
//! command is purely about identity material.

use super::format::ReportFormat;
use super::{CliError, CliResult};
use crate::protocol::{SensorId, SensorIdentity};
use clap::Subcommand;

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
        /// mandatory `0x`-prefixed 32-byte hex secret seed.
        uri: String,
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
/// secret) or a `0x`-prefixed 32-byte hex secret seed (reported with its
/// derived public key).
fn inspect(uri: String, output: ReportFormat) -> CliResult {
    let uri = uri.trim();

    // An SS58 address yields a public-only report (no secret is recoverable).
    if let Ok(sensor_id) = SensorId::from_ss58(uri) {
        return report(None, &sensor_id, output);
    }

    // Otherwise interpret the URI as a Substrate SURI: a mandatory
    // `0x`-prefixed hex secret seed or a BIP-39 recovery phrase, with
    // optional derivation junctions and password.
    let identity = SensorIdentity::from_suri(uri).map_err(|e| {
        CliError::usage(format!(
            "invalid key URI: expected an SS58 address or a valid SURI: {e}"
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
                println!("Secret seed:  {secret}");
            }
            println!("Public key:   {public_hex}");
            println!("SS58 Address: {ss58}");
        }
        ReportFormat::Json => {
            let mut map = serde_json::Map::new();
            if let Some(secret) = &secret_hex {
                map.insert("secretSeed".into(), serde_json::json!(secret));
            }
            map.insert("publicKey".into(), serde_json::json!(public_hex));
            map.insert("ss58Address".into(), serde_json::json!(ss58));
            let json = serde_json::to_string_pretty(&serde_json::Value::Object(map))
                .map_err(|e| CliError::runtime(format!("failed to serialize JSON: {e}")))?;
            println!("{json}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_reports_matching_ss58() {
        let identity = SensorIdentity::from_secret_bytes(&[7u8; 32]);
        // The rendered SS58 must round-trip back to the same account bytes.
        let ss58 = identity.sensor_id().to_ss58();
        let parsed = crate::protocol::SensorId::from_ss58(&ss58).unwrap();
        assert_eq!(parsed, identity.sensor_id());
    }
}
