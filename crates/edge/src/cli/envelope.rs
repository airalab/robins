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
//! `edge envelope` (alias `e`) — encode/decode the Connectivity Protocol
//! `crypto.v1.SignedEnvelope`.
//!
//! There is a single auto-detecting command instead of separate
//! `encode`/`decode` subcommands: the direction is chosen by sniffing the
//! input (a JSON document encodes, wire bytes in hex/base64/binary decode),
//! unless `--input`/`--output` force a specific representation. The inner
//! telemetry `message` is opaque here — see `edge message` for that payload.
//!
//! `--sign <SURI>` produces a fresh envelope from a raw message instead of
//! parsing one from `stdin`/`data`, and `--verify` checks an envelope's
//! Ed25519 signature (from either direction) before it is emitted.

use super::format::{self, ByteFormat, Direction, ReprFormat};
use super::{CliError, CliResult};
use crate::protocol::{self, SensorIdentity, SignOptions, SignedEnvelope};
use base64::Engine;
use clap::Args;
use serde::{Deserialize, Serialize};

/// `edge envelope` arguments.
#[derive(Debug, Args)]
#[command(after_help = "\
EXAMPLES:
    # Encode a JSON envelope (from stdin) to binary wire bytes on stdout.
    echo '{\"sensor_id\":\"0xaa..\",\"timestamp\":1700000000000,\"nonce\":\"0x00..\",\
\"message\":\"0xde..\",\"signature\":\"0xab..\"}' | edge envelope --output binary > envelope.bin

    # Decode wire bytes read from stdin to JSON; message_id is embedded.
    cat envelope.bin | edge envelope --output json

    # Decode a hex-encoded envelope passed positionally (direction and byte
    # format are both auto-detected, so `--input`/`--output` are optional).
    edge envelope 0x0a20aabb...

    # Round-trip through JSON, reproducing the exact original wire bytes.
    edge envelope --input hex --output json <envelope.hex \
      | edge e --input json --output hex

    # Sign a raw message read from stdin, producing a binary envelope.
    echo -n 'hello' | edge envelope --sign 0xd6a1...seed... > envelope.bin

    # Verify an envelope's signature while decoding it; exits non-zero (and
    # prints nothing to stdout) if the signature does not check out.
    cat envelope.bin | edge envelope --verify --output json")]
pub(crate) struct EnvelopeArgs {
    /// Envelope data, or (with `--sign`) the raw message to sign. If
    /// omitted, reads from stdin. Decoded per `--input` (or sniffed:
    /// hex/base64/binary wire bytes).
    data: Option<String>,
    /// Force the input representation (otherwise sniffed from content: a
    /// JSON document encodes, anything else decodes). With `--sign`, only
    /// `binary`/`base64`/`hex` are meaningful (default `binary`).
    #[arg(short, long, value_enum)]
    input: Option<ReprFormat>,
    /// Force the output representation (otherwise: `json` when decoding,
    /// `binary` when encoding or signing).
    #[arg(short, long, value_enum)]
    output: Option<ReprFormat>,
    /// Sign a raw message with this Ed25519 identity (a Substrate SURI: a
    /// `0x`-prefixed hex seed or a BIP-39 phrase, with optional derivation
    /// junctions), producing a fresh `SignedEnvelope` instead of parsing one.
    #[arg(long, value_name = "SURI")]
    sign: Option<String>,
    /// Measurement timestamp in Unix milliseconds, used with `--sign`
    /// (defaults to now).
    #[arg(long, requires = "sign")]
    timestamp: Option<u64>,
    /// Anti-replay nonce as `0x`-prefixed hex, used with `--sign` (defaults to
    /// a fresh random value).
    #[arg(long, requires = "sign")]
    nonce: Option<String>,
    /// Verify the envelope's Ed25519 signature before emitting it; exits
    /// with a non-zero status (and no stdout) if it does not verify.
    #[arg(long)]
    verify: bool,
}

/// JSON data-transfer object for a `SignedEnvelope`.
///
/// Byte fields are `0x`-prefixed hex so the JSON is stable and
/// round-trippable without adding serde derives to the generated prost types.
/// `message_id` is only ever present on output (the decode direction); it is
/// ignored if supplied on input.
#[derive(Debug, Serialize, Deserialize)]
struct EnvelopeJson {
    /// Sensor public key (32 bytes, `0x`-prefixed hex).
    sensor_id: String,
    /// Measurement timestamp, Unix milliseconds.
    timestamp: u64,
    /// Anti-replay nonce (`0x`-prefixed hex).
    nonce: String,
    /// Opaque telemetry payload (`0x`-prefixed hex).
    message: String,
    /// Ed25519 signature (64 bytes, `0x`-prefixed hex).
    signature: String,
}

impl EnvelopeJson {
    /// Build from a decoded envelope plus its dedup id.
    fn from_envelope(env: &SignedEnvelope) -> Self {
        Self {
            sensor_id: format!("0x{}", hex::encode(&env.sensor_id)),
            timestamp: env.timestamp,
            nonce: format!("0x{}", hex::encode(&env.nonce)),
            message: format!("0x{}", hex::encode(&env.message)),
            signature: format!("0x{}", hex::encode(&env.signature)),
        }
    }

    /// Convert into an envelope, hex-decoding each byte field. Each field must
    /// carry the mandatory `0x` prefix.
    fn into_envelope(self) -> Result<SignedEnvelope, CliError> {
        let decode = |field: &str, value: &str| {
            let stripped = crate::protocol::strip_0x(value.trim())
                .map_err(|e| CliError::with_code(3, format!("invalid hex in `{field}`: {e}")))?;
            hex::decode(stripped)
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

/// Dispatch `edge envelope`.
pub(crate) fn run(args: EnvelopeArgs) -> CliResult {
    let (env, wire, default_output) = if let Some(suri) = &args.sign {
        let (env, wire) = sign(suri, &args)?;
        (env, wire, ReprFormat::Binary)
    } else {
        decode_or_parse(&args)?
    };

    if args.verify {
        // Propagates the signature/protocol exit-code contract (`1` invalid,
        // `2`/`3` malformed) so scripts can distinguish failure modes.
        let verified = protocol::verify_envelope(&env).map_err(format::protocol_err)?;
        // Diagnostics only; stdout stays a clean data stream.
        eprintln!("signature: valid ({})", verified.sensor_id.to_ss58());
    }

    let output = args.output.unwrap_or(default_output);
    write_repr(&env, &wire, output)
}

/// Build the raw input bytes (positional `data` or stdin), for use before the
/// input has otherwise been consumed.
fn wire_or_data(args: &EnvelopeArgs) -> Result<Vec<u8>, CliError> {
    match &args.data {
        Some(text) => Ok(text.clone().into_bytes()),
        None => format::read_stdin(),
    }
}

/// Implements `edge envelope --sign`: read a raw message and produce a fresh
/// `SignedEnvelope`.
fn sign(suri: &str, args: &EnvelopeArgs) -> Result<(SignedEnvelope, Vec<u8>), CliError> {
    let identity = SensorIdentity::from_suri(suri)
        .map_err(|e| CliError::usage(format!("invalid --sign SURI: {e}")))?;

    let byte_format =
        match args.input {
            None | Some(ReprFormat::Binary) => ByteFormat::Binary,
            Some(ReprFormat::Base64) => ByteFormat::Base64,
            Some(ReprFormat::Hex) => ByteFormat::Hex,
            Some(ReprFormat::Json) => return Err(CliError::usage(
                "--input json is not valid with --sign; provide the raw message to sign instead",
            )),
        };
    let raw = wire_or_data(args)?;
    let message = format::wire_from_input(&raw, byte_format)?;

    let nonce = match &args.nonce {
        Some(hex_nonce) => Some({
            let stripped = crate::protocol::strip_0x(hex_nonce.trim())
                .map_err(|e| CliError::usage(format!("invalid --nonce hex: {e}")))?;
            hex::decode(stripped)
                .map_err(|e| CliError::usage(format!("invalid --nonce hex: {e}")))?
        }),
        None => None,
    };
    let options = SignOptions {
        timestamp_ms: args.timestamp,
        nonce,
    };
    let env = protocol::sign_message(&identity, &message, options);
    let wire = protocol::encode_envelope(&env);
    Ok((env, wire))
}

/// Decode an envelope from wire bytes, or parse one from JSON (the original
/// non-signing behaviour of `edge envelope`). Also returns the direction's
/// default output format (`json` when decoding, `binary` when encoding).
fn decode_or_parse(args: &EnvelopeArgs) -> Result<(SignedEnvelope, Vec<u8>, ReprFormat), CliError> {
    let raw = wire_or_data(args)?;
    let input = args.input.unwrap_or_else(|| sniff_repr(&raw));
    let direction = match input {
        ReprFormat::Json => Direction::Encode,
        ReprFormat::Binary | ReprFormat::Base64 | ReprFormat::Hex => Direction::Decode,
    };
    let default_output = match direction {
        Direction::Encode => ReprFormat::Binary,
        Direction::Decode => ReprFormat::Json,
    };

    let (env, wire) = match input {
        ReprFormat::Json => {
            let dto: EnvelopeJson = serde_json::from_slice(&raw)
                .map_err(|e| CliError::with_code(3, format!("invalid JSON: {e}")))?;
            let env = dto.into_envelope()?;
            let wire = protocol::encode_envelope(&env);
            (env, wire)
        }
        ReprFormat::Binary => decode_wire(&raw)?,
        ReprFormat::Base64 => decode_wire(&format::wire_from_input(&raw, ByteFormat::Base64)?)?,
        ReprFormat::Hex => decode_wire(&format::wire_from_input(&raw, ByteFormat::Hex)?)?,
    };

    if direction == Direction::Decode {
        // Diagnostics only; stdout stays a clean data stream for non-JSON
        // output so pipelines aren't corrupted.
        let message_id = protocol::envelope_id(&wire).to_hex();
        eprintln!("message_id: {message_id}");
    }

    Ok((env, wire, default_output))
}

/// Sniff the [`ReprFormat`] of `raw`: JSON if it parses as one, otherwise the
/// wire byte format (hex/base64/binary).
fn sniff_repr(raw: &[u8]) -> ReprFormat {
    match format::sniff_direction(raw) {
        Direction::Encode => ReprFormat::Json,
        Direction::Decode => match format::sniff_byte_format(raw) {
            ByteFormat::Binary => ReprFormat::Binary,
            ByteFormat::Base64 => ReprFormat::Base64,
            ByteFormat::Hex => ReprFormat::Hex,
        },
    }
}

/// Decode wire bytes into an envelope, preserving the original bytes.
fn decode_wire(wire: &[u8]) -> Result<(SignedEnvelope, Vec<u8>), CliError> {
    let env = protocol::decode_envelope(wire).map_err(format::protocol_err)?;
    Ok((env, wire.to_vec()))
}

/// Write an envelope in a [`ReprFormat`] to `stdout`. JSON output always
/// includes the dedup `message_id`.
fn write_repr(env: &SignedEnvelope, wire: &[u8], format: ReprFormat) -> CliResult {
    match format {
        ReprFormat::Binary => format::write_stdout(wire),
        ReprFormat::Base64 => {
            println!("{}", base64::engine::general_purpose::STANDARD.encode(wire));
            Ok(())
        }
        ReprFormat::Hex => {
            println!("0x{}", hex::encode(wire));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SensorIdentity, SignOptions};

    fn sample_identity() -> SensorIdentity {
        SensorIdentity::from_secret_bytes(&[7u8; 32])
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

    #[test]
    fn sniff_repr_detects_json_and_hex() {
        assert_eq!(sniff_repr(b"{\"a\":1}"), ReprFormat::Json);
        assert_eq!(sniff_repr(b"0xdeadbeef"), ReprFormat::Hex);
    }
}
