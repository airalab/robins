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
//! Shared stdin/stdout plumbing and wire-format types used by `edge envelope`,
//! `edge message` and `edge key`.
//!
//! This module owns no CLI subcommand of its own; it is purely the common
//! machinery (byte-format enums, content sniffing, stdin/stdout helpers, and
//! error mapping) that the command modules build on.

use super::CliError;
use crate::protocol::ProtocolError;
use base64::Engine;
use clap::ValueEnum;
use std::io::Read;

/// Wire representation for raw byte payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ByteFormat {
    /// Raw bytes.
    Binary,
    /// Base64 (standard alphabet, with padding).
    Base64,
    /// Lower-case hexadecimal, mandatory `0x` prefix.
    Hex,
}

/// Representation for structured messages: byte formats plus a
/// human-readable debug rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ReprFormat {
    /// Raw protobuf wire bytes.
    Binary,
    /// Base64-encoded protobuf wire bytes.
    Base64,
    /// Hex-encoded protobuf wire bytes, mandatory `0x` prefix.
    Hex,
    /// Human-readable pretty-printed debug rendering of the decoded
    /// protobuf struct (decode direction only).
    Text,
}

/// Read all bytes from `stdin`.
pub(crate) fn read_stdin() -> Result<Vec<u8>, CliError> {
    let mut buf = Vec::new();
    std::io::stdin()
        .read_to_end(&mut buf)
        .map_err(|e| CliError::runtime(format!("failed to read stdin: {e}")))?;
    Ok(buf)
}

/// Interpret raw bytes as wire bytes according to `format`.
pub(crate) fn wire_from_input(raw: &[u8], format: ByteFormat) -> Result<Vec<u8>, CliError> {
    match format {
        ByteFormat::Binary => Ok(raw.to_vec()),
        ByteFormat::Base64 => base64::engine::general_purpose::STANDARD
            .decode(trimmed(raw))
            .map_err(|e| CliError::with_code(3, format!("invalid base64 input: {e}"))),
        ByteFormat::Hex => {
            let text = std::str::from_utf8(trimmed(raw))
                .map_err(|_| CliError::with_code(3, "invalid hex input: not valid UTF-8"))?;
            let stripped = crate::protocol::strip_0x(text)
                .map_err(|e| CliError::with_code(3, format!("invalid hex input: {e}")))?;
            hex::decode(stripped)
                .map_err(|e| CliError::with_code(3, format!("invalid hex input: {e}")))
        }
    }
}

/// Trim ASCII whitespace (so trailing newlines from `echo`/pipes are tolerated).
pub(crate) fn trimmed(raw: &[u8]) -> &[u8] {
    let start = raw.iter().position(|b| !b.is_ascii_whitespace());
    let end = raw.iter().rposition(|b| !b.is_ascii_whitespace());
    match (start, end) {
        (Some(s), Some(e)) => &raw[s..=e],
        _ => &[],
    }
}

/// Write raw bytes to `stdout` (used for binary output).
pub(crate) fn write_stdout(bytes: &[u8]) -> crate::cli::CliResult {
    use std::io::Write;
    std::io::stdout()
        .write_all(bytes)
        .map_err(|e| CliError::runtime(format!("failed to write stdout: {e}")))
}

/// Map a [`ProtocolError`] onto a [`CliError`] using its exit-code contract.
pub(crate) fn protocol_err(err: ProtocolError) -> CliError {
    CliError::with_code(err.exit_code() as u8, err.to_string())
}

// ---------------------------------------------------------------------------
// Pretty text rendering
// ---------------------------------------------------------------------------
//
// Shared styling for the `text` output of `edge envelope`/`edge message`,
// matching the ASCII tree/bracket-tag look used by `libcps`'s CLI (see
// `libcps::display`): a bold heading, then `|--`/`` `-- `` branches with a
// short bracketed `[TAG]`. Colour is applied unconditionally here; `colored`
// itself decides whether to actually emit ANSI codes (it auto-detects a
// non-terminal stdout, and honours `NO_COLOR`/`CLICOLOR`, plus the explicit
// `--no-color` flag wired in `cli::run` via `colored::control::set_override`).

use colored::Colorize;

/// Print a bold heading line tagged `[TAG]` (e.g. the struct name being
/// rendered).
pub(crate) fn heading(tag: &str, title: impl std::fmt::Display) {
    println!("{} {}", tag_label(tag), title.to_string().bold());
}

/// Print one ASCII tree line at nesting level `indent` (0 = directly under
/// the heading), using a middle branch (`|--`) or final branch (`` `-- ``)
/// depending on `is_last`.
pub(crate) fn line(indent: usize, is_last: bool, tag: &str, text: impl std::fmt::Display) {
    let pad = "    ".repeat(indent);
    let glyph = if is_last { "`--" } else { "|--" };
    println!("{pad}{} {} {text}", glyph.bright_black(), tag_label(tag));
}

/// A short bracketed tag (e.g. `[S]`), styled consistently across lines.
fn tag_label(tag: &str) -> colored::ColoredString {
    format!("[{tag}]").bright_yellow().bold()
}

/// Sniff which [`ByteFormat`] `raw` is encoded in, for the decode direction.
///
/// Order of precedence: strict hex (mandatory `0x` prefix, even-length,
/// `[0-9a-fA-F]` remainder) first, then base64 (valid alphabet and decodes
/// cleanly), otherwise raw binary.
pub(crate) fn sniff_byte_format(raw: &[u8]) -> ByteFormat {
    let t = trimmed(raw);
    let is_hex = match t.strip_prefix(b"0x") {
        Some(digits) => {
            !digits.is_empty()
                && digits.len() % 2 == 0
                && digits.iter().all(|b| b.is_ascii_hexdigit())
        }
        None => false,
    };
    if is_hex {
        return ByteFormat::Hex;
    }
    let is_base64_alphabet = !t.is_empty()
        && t.iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/' || b == b'=');
    if is_base64_alphabet && base64::engine::general_purpose::STANDARD.decode(t).is_ok() {
        return ByteFormat::Base64;
    }
    ByteFormat::Binary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trimmed_strips_surrounding_whitespace() {
        assert_eq!(trimmed(b"  ab\n"), b"ab");
        assert_eq!(trimmed(b"\n\n"), b"");
    }

    #[test]
    fn wire_from_input_hex_roundtrip() {
        let out = wire_from_input(b"0xdeadbeef\n", ByteFormat::Hex).unwrap();
        assert_eq!(out, vec![0xde, 0xad, 0xbe, 0xef]);
    }

    #[test]
    fn wire_from_input_hex_requires_0x_prefix() {
        let err = wire_from_input(b"deadbeef", ByteFormat::Hex).unwrap_err();
        assert!(err.message.contains("0x"), "{}", err.message);
    }

    #[test]
    fn sniff_byte_format_prefers_hex_over_base64() {
        // "deadbeef" is valid hex and happens to also be a valid base64
        // alphabet string; a `0x`-prefixed value must be sniffed as hex.
        assert_eq!(sniff_byte_format(b"0xdeadbeef"), ByteFormat::Hex);
    }

    #[test]
    fn sniff_byte_format_requires_0x_prefix_for_hex() {
        // Without the `0x` prefix, "deadbeef" is not treated as hex; it falls
        // back to base64 (it happens to be valid in that alphabet too).
        assert_eq!(sniff_byte_format(b"deadbeef"), ByteFormat::Base64);
    }

    #[test]
    fn sniff_byte_format_detects_base64() {
        assert_eq!(sniff_byte_format(b"aGVsbG8="), ByteFormat::Base64);
    }

    #[test]
    fn sniff_byte_format_falls_back_to_binary() {
        assert_eq!(
            sniff_byte_format(&[0xff, 0x00, 0x01, 0x02]),
            ByteFormat::Binary
        );
    }
}
