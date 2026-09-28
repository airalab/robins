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
//! `edge batch` (alias `b`) — encode/decode the Connectivity Protocol
//! `crypto.v1.SignedEnvelopeBatch`.
//!
//! Like `edge envelope`, this is a single auto-detecting command rather than
//! separate `encode`/`decode` subcommands:
//!
//! - The normal (non-packing) path decodes wire bytes (hex/base64/binary,
//!   sniffed unless `--input` forces one) into a `SignedEnvelopeBatch`, and
//!   prints each contained [`SignedEnvelope`] as a human-readable rendering
//!   (or a byte representation with `--output`).
//! - `--pack`/`-p` builds a fresh batch instead: it reads one encoded
//!   envelope per line (positionally or from stdin), decodes each
//!   individually (sniffed per line unless `--input` forces one), and emits
//!   the packed `SignedEnvelopeBatch`.
//!
//! Unless `--output` forces a representation, the output defaults to a
//! human-readable debug rendering when stdout is a terminal (decode
//! direction only), and to raw wire bytes otherwise.
//!
//! `--decompress`/`-d` runs the decoded wire bytes through an XZ decoder
//! before parsing the protobuf `SignedEnvelopeBatch`, for batches that were
//! stored/transmitted XZ-compressed (e.g. pulled from IPFS; see
//! `src/protobufs/README.md`'s batch-anchoring flow). `--compress`/`-c` is
//! its `--pack` counterpart: it XZ-compresses the freshly packed batch
//! before writing it out.

use super::format::{self, ByteFormat, ReprFormat};
use super::{CliError, CliResult};
use crate::protocol::{self, SignedEnvelope, SignedEnvelopeBatch};
use base64::Engine;
use std::io::{IsTerminal, Read, Write};
use xz2::read::XzDecoder;
use xz2::write::XzEncoder;

use clap::Args;

/// `edge batch` arguments.
#[derive(Debug, Args)]
#[command(after_help = "\
EXAMPLES:
    # Decode a hex-encoded batch passed positionally (direction and byte
    # format are both auto-detected, so `--input`/`--output` are optional);
    # prints a human-readable rendering of each envelope when interactive.
    edge batch 0x0a20aabb...

    # Decode wire bytes read from stdin to hex.
    cat batch.bin | edge batch --output hex

    # Pack envelope lines (one encoded envelope per line, e.g. hex) read
    # from stdin into a single binary-encoded batch.
    printf '%s\\n%s\\n' \"$ENV1_HEX\" \"$ENV2_HEX\" | edge batch -p > batch.bin

    # Pack envelopes given positionally instead of via stdin.
    edge batch -p --output hex 0xaabb... 0xccdd...

    # Pack and XZ-compress in one step.
    printf '%s\\n%s\\n' \"$ENV1_HEX\" \"$ENV2_HEX\" | edge batch -p -c > batch.bin.xz

    # Decode an XZ-compressed batch (e.g. fetched from IPFS).
    cat batch.bin.xz | edge batch -d --output hex")]
pub(crate) struct BatchArgs {
    /// Batch data (decode direction), or one encoded envelope per value
    /// (with `--pack`). If omitted, reads from stdin (a single blob when
    /// decoding, or one envelope per line when packing).
    data: Vec<String>,
    /// Force the input representation (otherwise sniffed from content:
    /// hex/base64/binary wire bytes). With `--pack`, applies to each
    /// envelope individually.
    #[arg(short, long, value_enum)]
    input: Option<ReprFormat>,
    /// Force the output representation (otherwise: `text` when stdout is a
    /// terminal, `binary` otherwise; `--pack` never defaults to `text`).
    #[arg(short, long, value_enum)]
    output: Option<ReprFormat>,
    /// Pack individually-encoded envelopes (one per positional value, or one
    /// per stdin line) into a fresh `SignedEnvelopeBatch`, instead of
    /// decoding an existing batch.
    #[arg(short, long)]
    pack: bool,
    /// Decompress the decoded wire bytes (XZ) before parsing them as a
    /// `SignedEnvelopeBatch`. Only meaningful when decoding (ignored with
    /// `--pack`).
    #[arg(short = 'd', long)]
    decompress: bool,
    /// Compress the freshly packed `SignedEnvelopeBatch` (XZ) before writing
    /// it out. Only meaningful with `--pack`.
    #[arg(short = 'c', long, requires = "pack")]
    compress: bool,
}

/// Dispatch `edge batch`.
pub(crate) fn run(args: BatchArgs) -> CliResult {
    let batch = if args.pack {
        pack(&args)?
    } else {
        decode(&args)?
    };

    let mut wire = protocol::encode_envelope_batch(&batch);
    if args.compress {
        wire = compress_xz(&wire)?;
    }
    let output = args.output.unwrap_or_else(|| default_output(args.pack));
    write_repr(&batch, &wire, output)
}

/// Default output representation when `--output` is not given: a
/// human-readable debug rendering when stdout is a terminal and we are
/// decoding (raw wire bytes otherwise), or always raw wire bytes when
/// packing (packing produces data meant to be consumed downstream, not a
/// dump of what was just packed).
fn default_output(packing: bool) -> ReprFormat {
    if !packing && std::io::stdout().is_terminal() {
        ReprFormat::Text
    } else {
        ReprFormat::Binary
    }
}

/// Build the raw input bytes for the decode direction: a single positional
/// value (joined, though normally there is only one) or stdin.
fn wire_or_data(args: &BatchArgs) -> Result<Vec<u8>, CliError> {
    if args.data.is_empty() {
        format::read_stdin()
    } else {
        Ok(args.data.join("").into_bytes())
    }
}

/// Byte format to use for a single input value, honouring `--input` or
/// sniffing the content.
fn byte_format(args: &BatchArgs, raw: &[u8]) -> Result<ByteFormat, CliError> {
    match args.input {
        None => Ok(format::sniff_byte_format(raw)),
        Some(ReprFormat::Binary) => Ok(ByteFormat::Binary),
        Some(ReprFormat::Base64) => Ok(ByteFormat::Base64),
        Some(ReprFormat::Hex) => Ok(ByteFormat::Hex),
        Some(ReprFormat::Text) => Err(CliError::usage(
            "--input text is not supported for input; provide wire bytes (hex/base64/binary)",
        )),
    }
}

/// Implements `edge batch --pack`: read one encoded envelope per positional
/// value (or per stdin line), decode each, and pack them into a fresh
/// `SignedEnvelopeBatch`.
fn pack(args: &BatchArgs) -> Result<SignedEnvelopeBatch, CliError> {
    let lines: Vec<Vec<u8>> = if !args.data.is_empty() {
        args.data.iter().map(|s| s.clone().into_bytes()).collect()
    } else {
        let raw = format::read_stdin()?;
        raw.split(|&b| b == b'\n')
            .map(format::trimmed)
            .filter(|line| !line.is_empty())
            .map(|line| line.to_vec())
            .collect()
    };

    if lines.is_empty() {
        return Err(CliError::usage(
            "--pack requires at least one envelope (positionally, or one per stdin line)",
        ));
    }

    let mut envelopes = Vec::with_capacity(lines.len());
    for line in &lines {
        let format = byte_format(args, line)?;
        let wire = format::wire_from_input(line, format)?;
        let env = protocol::decode_envelope(&wire).map_err(format::protocol_err)?;
        envelopes.push(env);
    }

    Ok(SignedEnvelopeBatch { batch: envelopes })
}

/// Decompress `input` as an XZ stream (`--decompress`/`-d`).
fn decompress_xz(input: &[u8]) -> Result<Vec<u8>, CliError> {
    let mut out = Vec::new();
    XzDecoder::new(input)
        .read_to_end(&mut out)
        .map_err(|e| CliError::with_code(3, format!("invalid xz-compressed input: {e}")))?;
    Ok(out)
}

/// Compress `input` as an XZ stream (`--compress`/`-c`).
fn compress_xz(input: &[u8]) -> Result<Vec<u8>, CliError> {
    let mut encoder = XzEncoder::new(Vec::new(), 6);
    encoder
        .write_all(input)
        .map_err(|e| CliError::runtime(format!("failed to xz-compress output: {e}")))?;
    encoder
        .finish()
        .map_err(|e| CliError::runtime(format!("failed to xz-compress output: {e}")))
}

/// Decode a batch from wire bytes (the non-packing behaviour of `edge
/// batch`), sniffing the byte format unless `--input` forces one.
fn decode(args: &BatchArgs) -> Result<SignedEnvelopeBatch, CliError> {
    let raw = wire_or_data(args)?;
    let format = byte_format(args, &raw)?;
    let wire = format::wire_from_input(&raw, format)?;
    let wire = if args.decompress {
        decompress_xz(&wire)?
    } else {
        wire
    };
    protocol::decode_envelope_batch(&wire).map_err(format::protocol_err)
}

/// Write a batch in a [`ReprFormat`] to `stdout`.
fn write_repr(batch: &SignedEnvelopeBatch, wire: &[u8], format: ReprFormat) -> CliResult {
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
            format::heading(
                "B",
                format!("SignedEnvelopeBatch ({} envelopes)", batch.batch.len()),
            );
            let last = batch.batch.len().saturating_sub(1);
            for (i, env) in batch.batch.iter().enumerate() {
                write_envelope_text(i, env, i == last);
            }
            Ok(())
        }
    }
}

/// Write one envelope's fields as nested ASCII-tree lines under the batch
/// heading (mirrors `edge envelope`'s `text` rendering).
fn write_envelope_text(index: usize, env: &SignedEnvelope, is_last: bool) {
    format::line(0, is_last, "E", format!("envelope[{index}]:"));
    format::line(
        1,
        false,
        "#",
        format!("sensor_id:  0x{}", hex::encode(&env.sensor_id)),
    );
    format::line(
        1,
        false,
        "N",
        format!("nonce:      0x{}", hex::encode(&env.nonce)),
    );
    format::line(
        1,
        false,
        "M",
        format!("message:    0x{}", hex::encode(&env.message)),
    );
    format::line(
        1,
        true,
        "S",
        format!("signature:  0x{}", hex::encode(&env.signature)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{SensorIdentity, SignOptions};

    fn sample_identity(seed: u8) -> SensorIdentity {
        SensorIdentity::from_secret_bytes(&[seed; 32])
    }

    #[test]
    fn pack_and_decode_batch_roundtrip() {
        let a = protocol::sign_message(&sample_identity(1), b"a", SignOptions::default());
        let b = protocol::sign_message(&sample_identity(2), b"b", SignOptions::default());
        let batch = SignedEnvelopeBatch {
            batch: vec![a.clone(), b.clone()],
        };
        let wire = protocol::encode_envelope_batch(&batch);
        let decoded = protocol::decode_envelope_batch(&wire).unwrap();
        assert_eq!(decoded.batch.len(), 2);
        assert_eq!(decoded.batch[0], a);
        assert_eq!(decoded.batch[1], b);
    }

    #[test]
    fn byte_format_sniffs_hex_by_default() {
        let args = BatchArgs {
            data: vec![],
            input: None,
            output: None,
            pack: false,
            decompress: false,
            compress: false,
        };
        assert_eq!(byte_format(&args, b"0xdeadbeef").unwrap(), ByteFormat::Hex);
    }

    #[test]
    fn compress_then_decompress_xz_roundtrips() {
        let a = protocol::sign_message(&sample_identity(1), b"a", SignOptions::default());
        let batch = SignedEnvelopeBatch {
            batch: vec![a.clone()],
        };
        let wire = protocol::encode_envelope_batch(&batch);

        let compressed = compress_xz(&wire).unwrap();
        let decompressed = decompress_xz(&compressed).unwrap();
        assert_eq!(decompressed, wire);
        let decoded = protocol::decode_envelope_batch(&decompressed).unwrap();
        assert_eq!(decoded.batch, vec![a]);
    }
}
