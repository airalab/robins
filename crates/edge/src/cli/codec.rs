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
//! `edge codec` — encode, decode and inspect protocol messages.
//!
//! Signing and verification live under `edge key`; this group is purely about
//! wire-format conversions and inspection. The shared stdin/stdout plumbing and
//! format types defined here are also reused by the `key` command group.
//!
//! Every subcommand reads from `stdin` and writes to `stdout`, so they compose
//! into shell pipelines. Only the [`SignedEnvelope`] type is supported; the
//! inner telemetry `message` is opaque to the gateway and is never decoded here.

use super::{CliError, CliResult};
use crate::protocol::{self, ProtocolError, SensorIdentity, SignedEnvelope};
use base64::Engine;
use clap::{Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

/// The protobuf message a codec command operates on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum MessageType {
    /// Opaque telemetry payload (not supported: the gateway treats it as bytes).
    Message,
    /// Connectivity Protocol `crypto.v1.SignedEnvelope`.
    Envelope,
}

/// Wire representation for raw byte payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ByteFormat {
    /// Raw bytes.
    Binary,
    /// Base64 (standard alphabet, with padding).
    Base64,
    /// Lower-case hexadecimal.
    Hex,
}

/// Representation for structured messages: byte formats plus JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ReprFormat {
    /// Raw protobuf wire bytes.
    Binary,
    /// Base64-encoded protobuf wire bytes.
    Base64,
    /// Hex-encoded protobuf wire bytes.
    Hex,
    /// Structured JSON with hex-encoded byte fields.
    Json,
}

/// Rendering for report-style commands (`verify`, `inspect`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ReportFormat {
    /// Human-readable text.
    Text,
    /// Machine-readable JSON.
    Json,
}

/// `edge codec` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum CodecCommand {
    /// Convert a structured representation into protobuf wire bytes.
    Encode {
        /// Message type to operate on.
        #[arg(long = "type", value_enum, default_value_t = MessageType::Envelope)]
        message_type: MessageType,
        /// Input representation read from stdin.
        #[arg(long, value_enum, default_value_t = ReprFormat::Json)]
        input: ReprFormat,
        /// Output representation written to stdout.
        #[arg(long, value_enum, default_value_t = ReprFormat::Binary)]
        output: ReprFormat,
    },
    /// Decode protobuf wire bytes into a structured representation.
    Decode {
        /// Message type to operate on.
        #[arg(long = "type", value_enum, default_value_t = MessageType::Envelope)]
        message_type: MessageType,
        /// Input representation read from stdin.
        #[arg(long, value_enum, default_value_t = ReprFormat::Binary)]
        input: ReprFormat,
        /// Output representation written to stdout.
        #[arg(long, value_enum, default_value_t = ReprFormat::Json)]
        output: ReprFormat,
    },
    /// Inspect an envelope, reporting its protocol fields.
    Inspect {
        /// Format of the envelope read from stdin.
        #[arg(long, value_enum, default_value_t = ByteFormat::Binary)]
        input: ByteFormat,
        /// Report rendering.
        #[arg(long, value_enum, default_value_t = ReportFormat::Text)]
        output: ReportFormat,
    },
    /// Compute the deduplication id (SHA-256) of an envelope.
    Id {
        /// Format of the envelope read from stdin.
        #[arg(long, value_enum, default_value_t = ByteFormat::Binary)]
        input: ByteFormat,
    },
}

/// JSON data-transfer object for a `SignedEnvelope`.
///
/// Byte fields are hex-encoded so the JSON is stable and round-trippable without
/// adding serde derives to the generated prost types.
#[derive(Debug, Serialize, Deserialize)]
struct EnvelopeJson {
    /// Sensor public key (32 bytes, hex).
    sensor_id: String,
    /// Measurement timestamp, Unix milliseconds.
    timestamp: u64,
    /// Anti-replay nonce (hex).
    nonce: String,
    /// Opaque telemetry payload (hex).
    message: String,
    /// Ed25519 signature (64 bytes, hex).
    signature: String,
}

impl EnvelopeJson {
    /// Build from a decoded envelope.
    fn from_envelope(env: &SignedEnvelope) -> Self {
        Self {
            sensor_id: hex::encode(&env.sensor_id),
            timestamp: env.timestamp,
            nonce: hex::encode(&env.nonce),
            message: hex::encode(&env.message),
            signature: hex::encode(&env.signature),
        }
    }

    /// Convert into an envelope, hex-decoding each byte field.
    fn into_envelope(self) -> Result<SignedEnvelope, CliError> {
        let decode = |field: &str, value: &str| {
            hex::decode(value.trim())
                .map_err(|e| CliError::with_code(3, format!("invalid hex in `{field}`: {e}")))
        };
        Ok(SignedEnvelope {
            sensor_id: decode("sensor_id", &self.sensor_id)?,
            timestamp: self.timestamp,
            nonce: decode("nonce", &self.nonce)?,
            message: decode("message", &self.message)?,
            signature: decode("signature", &self.signature)?,
        })
    }
}

/// Dispatch a `codec` subcommand.
pub(crate) fn run(command: CodecCommand) -> CliResult {
    match command {
        CodecCommand::Encode {
            message_type,
            input,
            output,
        } => encode(message_type, input, output),
        CodecCommand::Decode {
            message_type,
            input,
            output,
        } => decode(message_type, input, output),
        CodecCommand::Inspect { input, output } => inspect(input, output),
        CodecCommand::Id { input } => id(input),
    }
}

/// Reject the unsupported `--type message` variant with a usage error.
fn require_envelope(message_type: MessageType) -> Result<(), CliError> {
    match message_type {
        MessageType::Envelope => Ok(()),
        MessageType::Message => Err(CliError::usage(
            "`--type message` is not supported: the telemetry payload is opaque to the gateway",
        )),
    }
}

/// Read all bytes from `stdin`.
pub(crate) fn read_stdin() -> Result<Vec<u8>, CliError> {
    let mut buf = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buf)
        .map_err(|e| CliError::runtime(format!("failed to read stdin: {e}")))?;
    Ok(buf)
}

/// Interpret raw stdin bytes as wire bytes according to `format`.
pub(crate) fn wire_from_input(raw: &[u8], format: ByteFormat) -> Result<Vec<u8>, CliError> {
    match format {
        ByteFormat::Binary => Ok(raw.to_vec()),
        ByteFormat::Base64 => base64::engine::general_purpose::STANDARD
            .decode(trimmed(raw))
            .map_err(|e| CliError::with_code(3, format!("invalid base64 input: {e}"))),
        ByteFormat::Hex => hex::decode(trimmed(raw))
            .map_err(|e| CliError::with_code(3, format!("invalid hex input: {e}"))),
    }
}

/// Trim ASCII whitespace (so trailing newlines from `echo`/pipes are tolerated).
fn trimmed(raw: &[u8]) -> &[u8] {
    let start = raw.iter().position(|b| !b.is_ascii_whitespace());
    let end = raw.iter().rposition(|b| !b.is_ascii_whitespace());
    match (start, end) {
        (Some(s), Some(e)) => &raw[s..=e],
        _ => &[],
    }
}

/// Write an envelope in a [`ReprFormat`] to `stdout`.
pub(crate) fn write_repr(env: &SignedEnvelope, wire: &[u8], format: ReprFormat) -> CliResult {
    match format {
        ReprFormat::Binary => write_stdout(wire),
        ReprFormat::Base64 => {
            println!("{}", base64::engine::general_purpose::STANDARD.encode(wire));
            Ok(())
        }
        ReprFormat::Hex => {
            println!("{}", hex::encode(wire));
            Ok(())
        }
        ReprFormat::Json => {
            let json = serde_json::to_string_pretty(&EnvelopeJson::from_envelope(env))
                .map_err(|e| CliError::runtime(format!("failed to serialize JSON: {e}")))?;
            println!("{json}");
            Ok(())
        }
    }
}

/// Write raw bytes to `stdout` (used for binary output).
pub(crate) fn write_stdout(bytes: &[u8]) -> CliResult {
    std::io::stdout()
        .write_all(bytes)
        .map_err(|e| CliError::runtime(format!("failed to write stdout: {e}")))
}

/// Map a [`ProtocolError`] onto a [`CliError`] using its exit-code contract.
pub(crate) fn protocol_err(err: ProtocolError) -> CliError {
    CliError::with_code(err.exit_code() as u8, err.to_string())
}

/// Decode an envelope from a [`ReprFormat`] input, returning both the parsed
/// envelope and the exact wire bytes it was decoded from.
fn read_envelope_repr(input: ReprFormat) -> Result<(SignedEnvelope, Vec<u8>), CliError> {
    let raw = read_stdin()?;
    match input {
        ReprFormat::Json => {
            let dto: EnvelopeJson = serde_json::from_slice(&raw)
                .map_err(|e| CliError::with_code(3, format!("invalid JSON: {e}")))?;
            let env = dto.into_envelope()?;
            let wire = protocol::encode_envelope(&env);
            Ok((env, wire))
        }
        ReprFormat::Binary => decode_wire(&raw),
        ReprFormat::Base64 => {
            let wire = wire_from_input(&raw, ByteFormat::Base64)?;
            decode_wire(&wire)
        }
        ReprFormat::Hex => {
            let wire = wire_from_input(&raw, ByteFormat::Hex)?;
            decode_wire(&wire)
        }
    }
}

/// Decode wire bytes into an envelope, preserving the original bytes.
fn decode_wire(wire: &[u8]) -> Result<(SignedEnvelope, Vec<u8>), CliError> {
    let env = protocol::decode_envelope(wire).map_err(protocol_err)?;
    Ok((env, wire.to_vec()))
}

/// Decode an envelope from a [`ByteFormat`] input, returning the parsed envelope
/// and the exact wire bytes (needed for id/inspect which hash received bytes).
pub(crate) fn read_envelope_bytes(
    input: ByteFormat,
) -> Result<(SignedEnvelope, Vec<u8>), CliError> {
    let raw = read_stdin()?;
    let wire = wire_from_input(&raw, input)?;
    decode_wire(&wire)
}

/// Implements `edge codec encode`.
fn encode(message_type: MessageType, input: ReprFormat, output: ReprFormat) -> CliResult {
    require_envelope(message_type)?;
    let (env, wire) = read_envelope_repr(input)?;
    write_repr(&env, &wire, output)
}

/// Implements `edge codec decode`.
fn decode(message_type: MessageType, input: ReprFormat, output: ReprFormat) -> CliResult {
    require_envelope(message_type)?;
    let (env, wire) = read_envelope_repr(input)?;
    write_repr(&env, &wire, output)
}

/// Load an Ed25519 identity from a key file (32 raw bytes, or hex text).
pub(crate) fn load_identity(path: &Path) -> Result<SensorIdentity, CliError> {
    let bytes = std::fs::read(path)
        .map_err(|e| CliError::usage(format!("failed to read key file {}: {e}", path.display())))?;
    if bytes.len() == 32 {
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&bytes);
        return Ok(SensorIdentity::from_secret_bytes(&secret));
    }
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| CliError::usage("key file is neither 32 raw bytes nor valid UTF-8 hex"))?;
    let text = text.trim().strip_prefix("0x").unwrap_or(text.trim());
    SensorIdentity::from_hex(text).map_err(|e| CliError::usage(format!("invalid key file: {e}")))
}

/// Implements `edge codec inspect`.
fn inspect(input: ByteFormat, output: ReportFormat) -> CliResult {
    let (env, wire) = read_envelope_bytes(input)?;
    let message_id = protocol::envelope_id(&wire);
    let sensor_id = protocol::validate_envelope(&env).ok();
    let signature_valid = protocol::verify_envelope(&env).is_ok();
    let sensor_display = sensor_id
        .as_ref()
        .map(|s| s.to_ss58())
        .unwrap_or_else(|| hex::encode(&env.sensor_id));

    match output {
        ReportFormat::Text => {
            println!("protocol:        connectivity.crypto.v1.SignedEnvelope");
            println!("sensor_id:       {sensor_display}");
            println!("timestamp:       {}", env.timestamp);
            println!("nonce_size:      {}", env.nonce.len());
            println!("message_size:    {}", env.message.len());
            println!("signature_size:  {}", env.signature.len());
            println!("encoded_size:    {}", wire.len());
            println!("message_id:      {}", message_id.to_hex());
            println!("signature_valid: {signature_valid}");
        }
        ReportFormat::Json => {
            let value = serde_json::json!({
                "protocol": "connectivity.crypto.v1.SignedEnvelope",
                "sensor_id": sensor_display,
                "timestamp": env.timestamp,
                "nonce_size": env.nonce.len(),
                "message_size": env.message.len(),
                "signature_size": env.signature.len(),
                "encoded_size": wire.len(),
                "message_id": message_id.to_hex(),
                "signature_valid": signature_valid,
            });
            let json = serde_json::to_string_pretty(&value)
                .map_err(|e| CliError::runtime(format!("failed to serialize JSON: {e}")))?;
            println!("{json}");
        }
    }
    Ok(())
}

/// Implements `edge codec id`.
fn id(input: ByteFormat) -> CliResult {
    let raw = read_stdin()?;
    let wire = wire_from_input(&raw, input)?;
    // `id` hashes the exact received bytes and does not require a valid envelope,
    // but we still confirm the payload is a well-formed envelope for safety.
    protocol::decode_envelope(&wire).map_err(protocol_err)?;
    println!("{}", protocol::envelope_id(&wire).to_hex());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::SignOptions;

    fn sample_identity() -> SensorIdentity {
        SensorIdentity::from_secret_bytes(&[7u8; 32])
    }

    #[test]
    fn trimmed_strips_surrounding_whitespace() {
        assert_eq!(trimmed(b"  ab\n"), b"ab");
        assert_eq!(trimmed(b"\n\n"), b"");
    }

    #[test]
    fn wire_from_input_hex_roundtrip() {
        let out = wire_from_input(b"deadbeef\n", ByteFormat::Hex).unwrap();
        assert_eq!(out, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn envelope_json_roundtrip() {
        let identity = sample_identity();
        let env = protocol::sign_message(
            &identity,
            b"hello",
            SignOptions {
                timestamp_ms: Some(1_700_000_000_000),
                nonce: Some(vec![9u8; 16]),
            },
        );
        let dto = EnvelopeJson::from_envelope(&env);
        let back = dto.into_envelope().unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn signed_envelope_verifies() {
        let identity = sample_identity();
        let env = protocol::sign_message(&identity, b"payload", SignOptions::default());
        let wire = protocol::encode_envelope(&env);
        let (decoded, _) = decode_wire(&wire).unwrap();
        let verified = protocol::verify_envelope(&decoded).unwrap();
        assert_eq!(verified.sensor_id, identity.sensor_id());
    }
}
