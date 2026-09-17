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
//! Envelopes are only ever produced by signing a raw message (`--sign`); the
//! normal (non-signing) path decodes wire bytes (hex/base64/binary, sniffed
//! unless `--input` forces one) back into a [`SignedEnvelope`]. The inner
//! telemetry `message` is opaque here — see `edge message` for that payload.
//!
//! `--verify` checks an envelope's Ed25519 signature (from either direction)
//! before it is emitted. Unless `--output` forces a representation, the
//! output defaults to a human-readable debug rendering when stdout is a
//! terminal, and to raw wire bytes otherwise (so pipelines keep working
//! without needing `--output binary`).

use super::format::{self, ByteFormat, ReprFormat};
use super::{CliError, CliResult};
use crate::protocol::{self, SensorIdentity, SignOptions, SignedEnvelope};
use base64::Engine;
use clap::Args;
use std::io::IsTerminal;

/// `edge envelope` arguments.
#[derive(Debug, Args)]
#[command(after_help = "\
EXAMPLES:
    # Decode a hex-encoded envelope passed positionally (direction and byte
    # format are both auto-detected, so `--input`/`--output` are optional);
    # prints a human-readable debug rendering when run interactively.
    edge envelope 0x0a20aabb...

    # Decode wire bytes read from stdin to hex.
    cat envelope.bin | edge envelope --output hex

    # Sign a raw message read from stdin, producing a binary envelope.
    echo -n 'hello' | edge envelope --sign 0xd6a1...seed... > envelope.bin

    # Verify an envelope's signature while decoding it; exits non-zero (and
    # prints nothing to stdout) if the signature does not check out.
    cat envelope.bin | edge envelope --verify --output hex

    # Print the envelope's dedup id (SHA-256 of the wire bytes) to stderr
    # while decoding.
    cat envelope.bin | edge envelope --id --output hex")]
pub(crate) struct EnvelopeArgs {
    /// Envelope data, or (with `--sign`) the raw message to sign. If
    /// omitted, reads from stdin. Decoded per `--input` (or sniffed:
    /// hex/base64/binary wire bytes).
    data: Option<String>,
    /// Force the input representation (otherwise sniffed from content:
    /// hex/base64/binary wire bytes). With `--sign`, only
    /// `binary`/`base64`/`hex` are meaningful (default `binary`).
    #[arg(short, long, value_enum)]
    input: Option<ReprFormat>,
    /// Force the output representation (otherwise: `text` when stdout is a
    /// terminal, `binary` otherwise).
    #[arg(short, long, value_enum)]
    output: Option<ReprFormat>,
    /// Sign a raw message with this Ed25519 identity (a Substrate SURI: a
    /// `0x`-prefixed hex seed or a BIP-39 phrase, with optional derivation
    /// junctions), producing a fresh `SignedEnvelope` instead of parsing one.
    #[arg(long, value_name = "SURI")]
    sign: Option<String>,
    /// Anti-replay nonce as `0x`-prefixed hex, used with `--sign` (defaults to
    /// a fresh random value).
    #[arg(long, requires = "sign")]
    nonce: Option<String>,
    /// Verify the envelope's Ed25519 signature before emitting it; exits
    /// with a non-zero status (and no stdout) if it does not verify.
    #[arg(long)]
    verify: bool,
    /// Print the envelope's dedup id (SHA-256 of the encoded wire bytes) to
    /// stderr as `message_id: <hex>`. Diagnostics only; stdout stays a clean
    /// data stream.
    #[arg(long)]
    id: bool,
}

/// Dispatch `edge envelope`.
pub(crate) fn run(args: EnvelopeArgs) -> CliResult {
    let (env, wire) = if let Some(suri) = &args.sign {
        sign(suri, &args)?
    } else {
        decode(&args)?
    };

    if args.id {
        // Diagnostics only; stdout stays a clean data stream.
        let message_id = protocol::envelope_id(&wire).to_hex();
        eprintln!("message_id: {message_id}");
    }

    if args.verify {
        // Propagates the signature/protocol exit-code contract (`1` invalid,
        // `2`/`3` malformed) so scripts can distinguish failure modes.
        let verified = protocol::verify_envelope(&env).map_err(format::protocol_err)?;
        // Diagnostics only; stdout stays a clean data stream.
        eprintln!("signature: valid ({})", verified.sensor_id.to_ss58());
    }

    let output = args.output.unwrap_or_else(default_output);
    write_repr(&env, &wire, output)
}

/// Default output representation when `--output` is not given: a
/// human-readable debug rendering when stdout is a terminal, raw wire bytes
/// otherwise (so pipelines keep working without needing `--output binary`).
fn default_output() -> ReprFormat {
    if std::io::stdout().is_terminal() {
        ReprFormat::Text
    } else {
        ReprFormat::Binary
    }
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
            Some(ReprFormat::Text) => return Err(CliError::usage(
                "--input text is not valid with --sign; provide the raw message to sign instead",
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
    let options = SignOptions { nonce };
    let env = protocol::sign_message(&identity, &message, options);
    let wire = protocol::encode_envelope(&env);
    Ok((env, wire))
}

/// Decode an envelope from wire bytes (the non-signing behaviour of `edge
/// envelope`), sniffing the byte format unless `--input` forces one.
fn decode(args: &EnvelopeArgs) -> Result<(SignedEnvelope, Vec<u8>), CliError> {
    let raw = wire_or_data(args)?;
    let byte_format =
        match args.input {
            None => format::sniff_byte_format(&raw),
            Some(ReprFormat::Binary) => ByteFormat::Binary,
            Some(ReprFormat::Base64) => ByteFormat::Base64,
            Some(ReprFormat::Hex) => ByteFormat::Hex,
            Some(ReprFormat::Text) => return Err(CliError::usage(
                "--input text is not supported for input; provide wire bytes (hex/base64/binary)",
            )),
        };
    let wire = format::wire_from_input(&raw, byte_format)?;
    decode_wire(&wire)
}

/// Decode wire bytes into an envelope, preserving the original bytes.
fn decode_wire(wire: &[u8]) -> Result<(SignedEnvelope, Vec<u8>), CliError> {
    let env = protocol::decode_envelope(wire).map_err(format::protocol_err)?;
    Ok((env, wire.to_vec()))
}

/// Write an envelope in a [`ReprFormat`] to `stdout`.
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
        ReprFormat::Text => {
            format::heading("E", "SignedEnvelope");
            format::line(
                0,
                false,
                "#",
                format!("sensor_id:  0x{}", hex::encode(&env.sensor_id)),
            );
            format::line(
                0,
                false,
                "N",
                format!("nonce:      0x{}", hex::encode(&env.nonce)),
            );
            format::line(
                0,
                false,
                "M",
                format!("message:    0x{}", hex::encode(&env.message)),
            );
            format::line(
                0,
                true,
                "S",
                format!("signature:  0x{}", hex::encode(&env.signature)),
            );
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
    fn signed_envelope_verifies() {
        let identity = sample_identity();
        let env = protocol::sign_message(&identity, b"payload", SignOptions::default());
        let wire = protocol::encode_envelope(&env);
        let (decoded, _) = decode_wire(&wire).unwrap();
        let verified = protocol::verify_envelope(&decoded).unwrap();
        assert_eq!(verified.sensor_id, identity.sensor_id());
    }

    #[test]
    fn sniff_byte_format_detects_hex() {
        assert_eq!(format::sniff_byte_format(b"0xdeadbeef"), ByteFormat::Hex);
    }
}
