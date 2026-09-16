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
//! `edge message` (alias `m`) — encode/decode the opaque `core.v1.Message`
//! telemetry payload carried inside a [`SignedEnvelope`]'s `message` field.
//!
//! Like `edge envelope`, this is a single auto-detecting command rather than
//! separate `encode`/`decode` subcommands. Decoding renders the message as a
//! human-readable debug dump of the decoded protobuf struct. Encoding accepts
//! a compact line-oriented grammar (one measurement per line) so telemetry
//! can be hand-written or scripted directly, without an intermediate DTO:
//!
//! ```text
//! [<section>:] [<recipient>/] [<board>/] [<sensor>.] <measurement>=<value>
//! ```
//!
//! - `<section>`: `public` (default, may be omitted) or `private`.
//! - `<recipient>`: SS58 or `0x`-prefixed hex public key; required (and only
//!   allowed) under `private`. Private measurements are grouped by recipient,
//!   serialized together, and encrypted once per recipient via `libcps`
//!   (`--suri` selects the sender identity).
//! - `<board>`: `urban` or `insight` — selects `Message.payload`. Optional
//!   when it can be inferred from `<sensor>` or from an unambiguous
//!   `<measurement>` name.
//! - `<sensor>`: e.g. `bme280`, `bme680`, `scd41`, `sds011`, `ics43434`.
//!   Optional when the (board, measurement) pair is unambiguous.
//! - `<measurement>`: the scalar field name (short aliases accepted, e.g.
//!   `temp` for `temperature`).
//! - GPS is a special case (three independent fields, not a oneof):
//!   `[board/]gps=<lat>,<lon>,<height_m>` (positional, one line per fix).
//!
//! [`SignedEnvelope`]: crate::protocol::SignedEnvelope

use super::format::{self, ByteFormat};
use super::{CliError, CliResult};
use crate::protocol::sensor_message::{Message, Meta, Payload};
use crate::protocol::{
    self, Bme280, Bme680, Co2, Encrypted, EncryptedInsight, EncryptedUrban, Gps, Humidity,
    Ics43434, Insight, InsightSensor, NoiseLevel, Pm10, Pm25, Pressure, Scd41, Sds011, SensorId,
    Temperature, Urban, UrbanSensor,
};
use clap::{Args, ValueEnum};
use prost::Message as _;
use std::io::IsTerminal;
use std::str::FromStr;

/// `edge message` arguments.
#[derive(Debug, Args)]
#[command(after_help = "\
EXAMPLES:
    # Encode one public measurement (board `urban` is inferred from the
    # `bme280` sensor); wire bytes are written to stdout as binary.
    echo 'bme280.temp=21.5' | edge message > message.bin

    # Multiple public measurements across one message; short aliases (e.g.
    # `temp`) and the positional GPS syntax are both supported.
    printf 'bme280.temp=21.5\\nbme280.humidity=44\\nurban/gps=51.5,-0.12,35\\n' \\
      | edge message --output hex

    # A private measurement, encrypted for its recipient with the sender's
    # key; private lines for the same recipient are grouped and encrypted
    # together as a single ciphertext.
    echo 'private:5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty/temp=21.5' \\
      | edge message --suri 0xd6a1...seed... --output hex

    # Decode a wire message (hex here) back into a human-readable debug dump.
    edge message <0xdeadbeef.. --input hex --output text")]
pub(crate) struct MessageArgs {
    /// Message data. If omitted, reads from stdin. Decoded per `--input` (or
    /// sniffed: hex/base64/binary wire bytes).
    data: Option<String>,
    /// Force the input representation (otherwise sniffed: line-grammar text
    /// encodes, anything else decodes).
    #[arg(short, long, value_enum)]
    input: Option<MessageFormat>,
    /// Force the output representation (otherwise: `text` when stdout is a
    /// terminal, `binary` otherwise).
    #[arg(short, long, value_enum)]
    output: Option<MessageFormat>,
    /// Identity used for encryption: sender when the line grammar contains
    /// `private:` entries (they are encrypted for their recipient), or
    /// recipient with `--decrypt`. A Substrate SURI (a `0x`-prefixed hex
    /// seed or a BIP-39 phrase, with optional derivation junctions).
    #[arg(short, long, value_name = "SURI")]
    suri: Option<String>,
    /// Sensor owner public key (SS58 or `0x`-prefixed hex) to set on the
    /// encoded message's `metadata.owner` field. Unset (no owner) by default.
    #[arg(long, value_name = "ADDRESS")]
    owner: Option<String>,
    /// Print the message's dedup id (SHA-256 of the encoded wire bytes) to
    /// stderr as `message_id: <hex>`. Diagnostics only; stdout stays a clean
    /// data stream.
    #[arg(long)]
    id: bool,
    /// Decrypt `private` measurement sections when decoding to text, using
    /// `--suri` as the recipient's private key (the sender's public key
    /// travels with each entry, so it isn't needed separately). Each entry
    /// is decrypted independently and printed alongside the message; entries
    /// this key can't open (e.g. addressed to someone else) are left
    /// ciphertext-only, with a warning on stderr. No effect on non-text
    /// output or when encoding.
    #[arg(short, long, requires = "suri")]
    decrypt: bool,
}

/// Input/output representation for `edge message`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum MessageFormat {
    /// Raw protobuf wire bytes.
    Binary,
    /// Base64-encoded protobuf wire bytes.
    Base64,
    /// Hex-encoded protobuf wire bytes.
    Hex,
    /// Human-readable pretty-printed debug rendering of the decoded
    /// protobuf struct (decode direction only).
    Text,
    /// Compact line grammar (encode only; see module docs).
    Grammar,
}

/// Dispatch `edge message`.
pub(crate) fn run(args: MessageArgs) -> CliResult {
    let raw = match &args.data {
        Some(text) => text.clone().into_bytes(),
        None => format::read_stdin()?,
    };

    let input = args.input.unwrap_or_else(|| sniff_input(&raw));

    let wire = match input {
        MessageFormat::Grammar => {
            let text = std::str::from_utf8(&raw)
                .map_err(|_| CliError::usage("grammar input must be valid UTF-8 text"))?;
            let mut msg = grammar::parse(text, args.suri.as_deref())?;
            apply_owner(&mut msg, args.owner.as_deref())?;
            protocol::encode_sensor_message(&msg)
        }
        MessageFormat::Binary => raw.clone(),
        MessageFormat::Base64 => format::wire_from_input(&raw, ByteFormat::Base64)?,
        MessageFormat::Hex => format::wire_from_input(&raw, ByteFormat::Hex)?,
        MessageFormat::Text => {
            return Err(CliError::usage(
                "--input text is not supported; provide wire bytes (hex/base64/binary) or the line grammar",
            ))
        }
    };

    let output = args.output.unwrap_or_else(default_output);

    if args.id {
        // Diagnostics only; stdout stays a clean data stream.
        let message_id = protocol::envelope_id(&wire).to_hex();
        eprintln!("message_id: {message_id}");
    }

    let decrypt_suri = if args.decrypt {
        args.suri.as_deref()
    } else {
        None
    };
    write_output(&wire, output, decrypt_suri)
}

/// Default output representation when `--output` is not given: a
/// human-readable debug rendering when stdout is a terminal, raw wire bytes
/// otherwise (so pipelines keep working without needing `--output binary`).
/// Applies whether the input was decoded (wire bytes) or just encoded (the
/// line grammar): a fresh interactive `edge message` shows what was built.
fn default_output() -> MessageFormat {
    if std::io::stdout().is_terminal() {
        MessageFormat::Text
    } else {
        MessageFormat::Binary
    }
}

/// Sniff the [`MessageFormat`] of `raw`: the line grammar if it looks like
/// `key=value` text, else wire bytes.
fn sniff_input(raw: &[u8]) -> MessageFormat {
    if let Ok(text) = std::str::from_utf8(raw) {
        let looks_like_grammar = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .any(|l| l.contains('='));
        if looks_like_grammar {
            return MessageFormat::Grammar;
        }
    }
    match format::sniff_byte_format(raw) {
        ByteFormat::Binary => MessageFormat::Binary,
        ByteFormat::Base64 => MessageFormat::Base64,
        ByteFormat::Hex => MessageFormat::Hex,
    }
}

/// Write wire bytes decoded/converted to the requested [`MessageFormat`].
/// `decrypt_suri`, if set, is used to attempt decryption of `private`
/// measurement sections when writing text (see [`print_decrypted`]).
fn write_output(wire: &[u8], format: MessageFormat, decrypt_suri: Option<&str>) -> CliResult {
    use base64::Engine;
    match format {
        MessageFormat::Binary => format::write_stdout(wire),
        MessageFormat::Base64 => {
            println!("{}", base64::engine::general_purpose::STANDARD.encode(wire));
            Ok(())
        }
        MessageFormat::Hex => {
            println!("0x{}", hex::encode(wire));
            Ok(())
        }
        MessageFormat::Text => {
            let msg = protocol::decode_sensor_message(wire).map_err(format::protocol_err)?;
            print_message(&msg, decrypt_suri);
            Ok(())
        }
        MessageFormat::Grammar => Err(CliError::usage(
            "`--output grammar` is not supported: the line grammar is encode-only",
        )),
    }
}

/// Pretty-print a decoded `core.v1.Message` as an ASCII tree. If
/// `decrypt_suri` is given, attempts to decrypt each `private` entry using it
/// as the recipient's private key (the sender's public key travels with each
/// entry, so it isn't needed separately); entries this key can't open (e.g.
/// addressed to someone else) are left ciphertext-only, with a warning on
/// stderr rather than aborting the whole command.
fn print_message(msg: &Message, decrypt_suri: Option<&str>) {
    format::heading("M", "Message");
    if let Some(owner) = msg
        .metadata
        .as_ref()
        .map(|m| &m.owner)
        .filter(|o| !o.is_empty())
    {
        format::line(0, false, "O", format!("owner: 0x{}", hex::encode(owner)));
    }
    match msg.payload.as_ref() {
        Some(Payload::Urban(urban)) => print_urban(urban, decrypt_suri),
        Some(Payload::Insight(insight)) => print_insight(insight, decrypt_suri),
        None => format::line(0, true, "P", "(no payload)"),
    }
}

/// Print the `urban` board's public/private sensor readings.
fn print_urban(urban: &Urban, decrypt_suri: Option<&str>) {
    let total = urban.public.len() + urban.private.len();
    let mut n = 0;
    for sensor in &urban.public {
        n += 1;
        format::line(
            0,
            n == total,
            "S",
            format!("public:  {}", format_urban_sensor(sensor)),
        );
    }
    for entry in &urban.private {
        n += 1;
        print_private_entry::<EncryptedUrban, _>(
            n == total,
            entry,
            decrypt_suri,
            format_urban_sensor,
        );
    }
}

/// Print the `insight` board's public/private sensor readings.
fn print_insight(insight: &Insight, decrypt_suri: Option<&str>) {
    let total = insight.public.len() + insight.private.len();
    let mut n = 0;
    for sensor in &insight.public {
        n += 1;
        format::line(
            0,
            n == total,
            "S",
            format!("public:  {}", format_insight_sensor(sensor)),
        );
    }
    for entry in &insight.private {
        n += 1;
        print_private_entry::<EncryptedInsight, _>(
            n == total,
            entry,
            decrypt_suri,
            format_insight_sensor,
        );
    }
}

/// Print one `private` entry line (ciphertext metadata), then, if
/// `decrypt_suri` is given, attempt decryption and print the recovered
/// sensors indented underneath — formatted with `format_sensor` (the same
/// renderer used for public readings of that board).
fn print_private_entry<M, S>(
    is_last: bool,
    entry: &Encrypted,
    decrypt_suri: Option<&str>,
    format_sensor: impl Fn(&S) -> String,
) where
    M: prost::Message + Default + HasSensors<S>,
{
    let decrypted = decrypt_suri.map(|suri| decrypt_entry::<M>(entry, suri));
    format::line(
        0,
        is_last && decrypted.is_none(),
        "E",
        format!(
            "private: from=0x{} algorithm={}",
            hex::encode(&entry.from),
            entry.algorithm
        ),
    );
    match decrypted {
        Some(Ok(decoded)) => {
            let sensors = decoded.sensors();
            for (i, sensor) in sensors.iter().enumerate() {
                format::line(
                    1,
                    is_last && i + 1 == sensors.len(),
                    "D",
                    format!("decrypted: {}", format_sensor(sensor)),
                );
            }
        }
        Some(Err(e)) => eprintln!(
            "warning: failed to decrypt private entry from 0x{}: {e:?}",
            hex::encode(&entry.from)
        ),
        None => {}
    }
}

/// Accessor trait bridging the board-specific `EncryptedUrban`/
/// `EncryptedInsight` protobuf wrappers to their common shape (a `sensors`
/// field), so [`print_private_entry`] can stay generic over both boards
/// without introducing a JSON-style intermediate representation.
trait HasSensors<S> {
    fn sensors(&self) -> &[S];
}

impl HasSensors<UrbanSensor> for EncryptedUrban {
    fn sensors(&self) -> &[UrbanSensor] {
        &self.sensors
    }
}

impl HasSensors<InsightSensor> for EncryptedInsight {
    fn sensors(&self) -> &[InsightSensor] {
        &self.sensors
    }
}

/// Render one `device.v1.UrbanSensor` reading as a short human-readable
/// string, e.g. `bme280 temperature=21.50°C`.
fn format_urban_sensor(s: &UrbanSensor) -> String {
    use crate::protocol::generated::device::v1::urban_sensor::Sensor;
    match s.sensor.as_ref() {
        Some(Sensor::Gps(g)) => format_gps(g),
        Some(Sensor::Bme280(b)) => format!("bme280 {}", format_bme280(b)),
        Some(Sensor::Sds011(s)) => format!("sds011 {}", format_sds011(s)),
        Some(Sensor::Ics43434(i)) => format!("ics43434 {}", format_ics43434(i)),
        None => "(unset)".to_string(),
    }
}

/// Render one `device.v1.InsightSensor` reading, mirroring
/// [`format_urban_sensor`].
fn format_insight_sensor(s: &InsightSensor) -> String {
    use crate::protocol::generated::device::v1::insight_sensor::Sensor;
    match s.sensor.as_ref() {
        Some(Sensor::Gps(g)) => format_gps(g),
        Some(Sensor::Bme680(b)) => format!("bme680 {}", format_bme680(b)),
        Some(Sensor::Scd41(s)) => format!("scd41 {}", format_scd41(s)),
        None => "(unset)".to_string(),
    }
}

fn format_gps(g: &Gps) -> String {
    format!(
        "gps lat={:.5} lon={:.5} height_m={:.1}",
        g.lat, g.lon, g.height_m
    )
}

fn format_bme280(b: &Bme280) -> String {
    use crate::protocol::generated::sensor::v1::bme280::Measurement;
    match b.measurement.as_ref() {
        Some(Measurement::Temperature(t)) => format!("temperature={:.2}°C", t.celsius),
        Some(Measurement::Humidity(h)) => format!("humidity={:.2}%", h.percent),
        Some(Measurement::Pressure(p)) => format!("pressure={:.2}Pa", p.pascal),
        None => "(unset)".to_string(),
    }
}

fn format_bme680(b: &Bme680) -> String {
    use crate::protocol::generated::sensor::v1::bme680::Measurement;
    match b.measurement.as_ref() {
        Some(Measurement::Temperature(t)) => format!("temperature={:.2}°C", t.celsius),
        Some(Measurement::Humidity(h)) => format!("humidity={:.2}%", h.percent),
        Some(Measurement::Pressure(p)) => format!("pressure={:.2}Pa", p.pascal),
        None => "(unset)".to_string(),
    }
}

fn format_scd41(s: &Scd41) -> String {
    use crate::protocol::generated::sensor::v1::scd41::Measurement;
    match s.measurement.as_ref() {
        Some(Measurement::Co2(c)) => format!("co2={:.0}ppm", c.ppm),
        Some(Measurement::Temperature(t)) => format!("temperature={:.2}°C", t.celsius),
        Some(Measurement::Humidity(h)) => format!("humidity={:.2}%", h.percent),
        None => "(unset)".to_string(),
    }
}

fn format_sds011(s: &Sds011) -> String {
    use crate::protocol::generated::sensor::v1::sds011::Measurement;
    match s.measurement.as_ref() {
        Some(Measurement::Pm25(p)) => format!("pm25={:.1}ug/m3", p.ug_m3),
        Some(Measurement::Pm10(p)) => format!("pm10={:.1}ug/m3", p.ug_m3),
        None => "(unset)".to_string(),
    }
}

fn format_ics43434(i: &Ics43434) -> String {
    use crate::protocol::generated::sensor::v1::ics43434::Measurement;
    match i.measurement.as_ref() {
        Some(Measurement::NoiseMax(n)) => format!("noise_max={:.1}dB", n.db),
        Some(Measurement::NoiseAvg(n)) => format!("noise_avg={:.1}dB", n.db),
        None => "(unset)".to_string(),
    }
}

/// Set `msg.metadata.owner` from `--owner`, if given (SS58 or `0x`-prefixed
/// hex). Leaves `metadata` untouched (`None` by default) when `owner` is
/// `None`.
fn apply_owner(msg: &mut Message, owner: Option<&str>) -> Result<(), CliError> {
    if let Some(owner) = owner {
        msg.metadata = Some(Meta {
            owner: resolve_recipient(owner)?.to_vec(),
        });
    }
    Ok(())
}

/// Resolve an SS58 or `0x`-prefixed hex-encoded recipient string to its raw
/// public key.
fn resolve_recipient(recipient: &str) -> Result<[u8; 32], CliError> {
    let id = SensorId::from_ss58(recipient)
        .or_else(|_| SensorId::from_hex(recipient))
        .map_err(|e| CliError::usage(format!("invalid recipient `{recipient}`: {e}")))?;
    Ok(*id.as_bytes())
}

/// Decrypt one `private` entry's ciphertext with `suri` as the recipient's
/// private key, then decode the resulting plaintext as `M` — the
/// board-specific `EncryptedUrban`/`EncryptedInsight` protobuf wrapper
/// declared alongside the encoder in [`grammar::encrypt_group`].
fn decrypt_entry<M: prost::Message + Default>(
    entry: &Encrypted,
    suri: &str,
) -> Result<M, CliError> {
    let plaintext = decrypt_ciphertext(entry, suri)?;
    M::decode(plaintext.as_slice())
        .map_err(|e| CliError::with_code(3, format!("invalid decrypted payload: {e}")))
}

/// Decrypt an `Encrypted` entry's ciphertext with `suri` as the recipient's
/// private key, returning the raw plaintext bytes. The sender's public key
/// (`entry.from`) is used as-is; there is no separate sender verification
/// since it is exactly what identifies the entry as "self-describing".
fn decrypt_ciphertext(entry: &Encrypted, suri: &str) -> Result<Vec<u8>, CliError> {
    let algorithm =
        libcps::crypto::EncryptionAlgorithm::from_str(&entry.algorithm).map_err(|e| {
            CliError::runtime(format!(
                "unsupported encryption algorithm `{}`: {e}",
                entry.algorithm
            ))
        })?;
    let from: [u8; 32] = entry
        .from
        .as_slice()
        .try_into()
        .map_err(|_| CliError::with_code(3, "private entry `from` must be 32 bytes"))?;
    let message = libcps::crypto::EncryptedMessage::V1 {
        algorithm,
        from,
        nonce: entry.nonce.clone(),
        ciphertext: entry.ciphertext.clone(),
    };
    let cipher =
        libcps::crypto::Cipher::new(suri.to_string(), libcps::crypto::CryptoScheme::Ed25519)
            .map_err(|e| CliError::runtime(format!("failed to initialise cipher: {e}")))?;
    cipher
        .decrypt(&message, None)
        .map_err(|e| CliError::runtime(format!("decryption failed: {e}")))
}

// ---------------------------------------------------------------------------
// Compact line grammar
// ---------------------------------------------------------------------------

mod grammar {
    use super::*;

    /// Device board — selects `Message.payload`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Board {
        Urban,
        Insight,
    }

    impl Board {
        fn parse(s: &str) -> Option<Self> {
            match s {
                "urban" => Some(Self::Urban),
                "insight" => Some(Self::Insight),
                _ => None,
            }
        }
    }

    /// Sensor kind — the source device for a measurement.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum SensorKind {
        Gps,
        Bme280,
        Bme680,
        Scd41,
        Sds011,
        Ics43434,
    }

    /// All sensor kinds capable of producing scalar measurements (excludes
    /// `Gps`, which is handled through its own positional syntax).
    const SCALAR_SENSORS: &[SensorKind] = &[
        SensorKind::Bme280,
        SensorKind::Bme680,
        SensorKind::Scd41,
        SensorKind::Sds011,
        SensorKind::Ics43434,
    ];

    impl SensorKind {
        fn parse(s: &str) -> Option<Self> {
            match s {
                "gps" => Some(Self::Gps),
                "bme280" => Some(Self::Bme280),
                "bme680" => Some(Self::Bme680),
                "scd41" => Some(Self::Scd41),
                "sds011" => Some(Self::Sds011),
                "ics43434" => Some(Self::Ics43434),
                _ => None,
            }
        }

        /// The fixed board for this sensor, or `None` for `Gps` (valid on
        /// both boards).
        fn board(self) -> Option<Board> {
            match self {
                Self::Bme280 | Self::Sds011 | Self::Ics43434 => Some(Board::Urban),
                Self::Bme680 | Self::Scd41 => Some(Board::Insight),
                Self::Gps => None,
            }
        }

        /// Scalar measurement names this sensor can produce.
        fn measurements(self) -> &'static [&'static str] {
            match self {
                Self::Bme280 | Self::Bme680 => &["temperature", "humidity", "pressure"],
                Self::Scd41 => &["co2", "temperature", "humidity"],
                Self::Sds011 => &["pm25", "pm10"],
                Self::Ics43434 => &["noise_max", "noise_avg"],
                Self::Gps => &[],
            }
        }
    }

    /// Resolve a short measurement alias to its canonical name.
    fn canonical_measurement(name: &str) -> &str {
        match name {
            "temp" => "temperature",
            other => other,
        }
    }

    /// A single resolved value carried by a grammar line.
    enum Value {
        Measurement { name: &'static str, value: f64 },
        Gps { lat: f64, lon: f64, height_m: f64 },
    }

    /// One fully-resolved grammar line.
    struct Line {
        recipient: Option<String>,
        board: Board,
        kind: SensorKind,
        value: Value,
    }

    /// Parse the full grammar text into a `core.v1.Message`.
    ///
    /// `suri` is required (and used) only if the text contains any
    /// `private:` lines.
    pub(super) fn parse(text: &str, suri: Option<&str>) -> Result<Message, CliError> {
        let mut lines = Vec::new();
        for (n, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            lines.push(
                parse_line(line).map_err(|e| CliError::usage(format!("line {}: {e}", n + 1)))?,
            );
        }
        if lines.is_empty() {
            return Err(CliError::usage("no measurements provided"));
        }

        let board = lines[0].board;
        if lines.iter().any(|l| l.board != board) {
            return Err(CliError::usage(
                "all lines must resolve to the same board (urban/insight); got a mix",
            ));
        }

        let mut public_urban = Vec::new();
        let mut public_insight = Vec::new();
        // Recipient groups, keyed by recipient string, preserving first-seen order.
        let mut private_urban: Vec<(String, Vec<UrbanSensor>)> = Vec::new();
        let mut private_insight: Vec<(String, Vec<InsightSensor>)> = Vec::new();

        for line in lines {
            match (line.recipient, board) {
                (None, Board::Urban) => public_urban.push(build_urban(line.kind, &line.value)?),
                (None, Board::Insight) => {
                    public_insight.push(build_insight(line.kind, &line.value)?)
                }
                (Some(recipient), Board::Urban) => {
                    let entry = build_urban(line.kind, &line.value)?;
                    group_push(&mut private_urban, recipient, entry);
                }
                (Some(recipient), Board::Insight) => {
                    let entry = build_insight(line.kind, &line.value)?;
                    group_push(&mut private_insight, recipient, entry);
                }
            }
        }

        let identity = if !private_urban.is_empty() || !private_insight.is_empty() {
            let suri = suri.ok_or_else(|| {
                CliError::usage("`--suri` is required to encrypt `private:` measurements")
            })?;
            Some(
                protocol::SensorIdentity::from_suri(suri)
                    .map_err(|e| CliError::usage(format!("invalid --suri: {e}")))?,
            )
        } else {
            None
        };

        let payload = match board {
            Board::Urban => {
                let private = private_urban
                    .into_iter()
                    .map(|(recipient, sensors)| {
                        let plaintext = protocol::generated::device::v1::EncryptedUrban { sensors }
                            .encode_to_vec();
                        encrypt_group(
                            identity.as_ref().expect("checked above"),
                            &recipient,
                            plaintext,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Payload::Urban(Urban {
                    public: public_urban,
                    private,
                })
            }
            Board::Insight => {
                let private = private_insight
                    .into_iter()
                    .map(|(recipient, sensors)| {
                        let plaintext =
                            protocol::generated::device::v1::EncryptedInsight { sensors }
                                .encode_to_vec();
                        encrypt_group(
                            identity.as_ref().expect("checked above"),
                            &recipient,
                            plaintext,
                        )
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Payload::Insight(Insight {
                    public: public_insight,
                    private,
                })
            }
        };

        Ok(Message {
            metadata: None,
            payload: Some(payload),
        })
    }

    /// Push `entry` onto the group for `recipient`, creating it if needed.
    fn group_push<T>(groups: &mut Vec<(String, Vec<T>)>, recipient: String, entry: T) {
        if let Some((_, sensors)) = groups.iter_mut().find(|(r, _)| *r == recipient) {
            sensors.push(entry);
        } else {
            groups.push((recipient, vec![entry]));
        }
    }

    /// Encrypt one recipient's plaintext (already wrapped and serialized to
    /// protobuf wire bytes) for `recipient`, producing an `Encrypted` entry.
    fn encrypt_group(
        identity: &protocol::SensorIdentity,
        recipient: &str,
        plaintext: Vec<u8>,
    ) -> Result<Encrypted, CliError> {
        let receiver_public = super::resolve_recipient(recipient)?;
        let suri = identity.secret_to_hex();
        let cipher = libcps::crypto::Cipher::new(suri, libcps::crypto::CryptoScheme::Ed25519)
            .map_err(|e| CliError::runtime(format!("failed to initialise cipher: {e}")))?;
        let algorithm = libcps::crypto::EncryptionAlgorithm::XChaCha20Poly1305;
        let encrypted = cipher
            .encrypt(&plaintext, &receiver_public, algorithm)
            .map_err(|e| CliError::runtime(format!("encryption failed for `{recipient}`: {e}")))?;
        match encrypted {
            libcps::crypto::EncryptedMessage::V1 {
                from,
                nonce,
                ciphertext,
                ..
            } => Ok(Encrypted {
                version: 1,
                // Short algorithm tag, matching the Encrypted.algorithm proto
                // convention ("xchacha20"/"aesgcm256"/"chacha20"), distinct
                // from libcps's own HKDF `info_string()`.
                algorithm: "xchacha20".to_string(),
                from: from.to_vec(),
                nonce,
                ciphertext,
            }),
        }
    }

    fn build_urban(kind: SensorKind, value: &Value) -> Result<UrbanSensor, CliError> {
        use crate::protocol::generated::device::v1::urban_sensor::Sensor;
        let sensor = match (kind, value) {
            (SensorKind::Gps, Value::Gps { lat, lon, height_m }) => Sensor::Gps(Gps {
                lat: *lat,
                lon: *lon,
                height_m: *height_m,
            }),
            (SensorKind::Bme280, Value::Measurement { name, value }) => {
                Sensor::Bme280(bme280_measurement(name, *value)?)
            }
            (SensorKind::Sds011, Value::Measurement { name, value }) => {
                Sensor::Sds011(sds011_measurement(name, *value)?)
            }
            (SensorKind::Ics43434, Value::Measurement { name, value }) => {
                Sensor::Ics43434(ics43434_measurement(name, *value)?)
            }
            _ => return Err(CliError::usage("sensor/value mismatch for urban board")),
        };
        Ok(UrbanSensor {
            sensor: Some(sensor),
        })
    }

    fn build_insight(kind: SensorKind, value: &Value) -> Result<InsightSensor, CliError> {
        use crate::protocol::generated::device::v1::insight_sensor::Sensor;
        let sensor = match (kind, value) {
            (SensorKind::Gps, Value::Gps { lat, lon, height_m }) => Sensor::Gps(Gps {
                lat: *lat,
                lon: *lon,
                height_m: *height_m,
            }),
            (SensorKind::Bme680, Value::Measurement { name, value }) => {
                Sensor::Bme680(bme680_measurement(name, *value)?)
            }
            (SensorKind::Scd41, Value::Measurement { name, value }) => {
                Sensor::Scd41(scd41_measurement(name, *value)?)
            }
            _ => return Err(CliError::usage("sensor/value mismatch for insight board")),
        };
        Ok(InsightSensor {
            sensor: Some(sensor),
        })
    }

    fn bme280_measurement(name: &str, value: f64) -> Result<Bme280, CliError> {
        use crate::protocol::generated::sensor::v1::bme280::Measurement;
        let measurement = match name {
            "temperature" => Measurement::Temperature(Temperature { celsius: value }),
            "humidity" => Measurement::Humidity(Humidity { percent: value }),
            "pressure" => Measurement::Pressure(Pressure { pascal: value }),
            other => {
                return Err(CliError::usage(format!(
                    "bme280 has no measurement `{other}`"
                )))
            }
        };
        Ok(Bme280 {
            measurement: Some(measurement),
        })
    }

    fn bme680_measurement(name: &str, value: f64) -> Result<Bme680, CliError> {
        use crate::protocol::generated::sensor::v1::bme680::Measurement;
        let measurement = match name {
            "temperature" => Measurement::Temperature(Temperature { celsius: value }),
            "humidity" => Measurement::Humidity(Humidity { percent: value }),
            "pressure" => Measurement::Pressure(Pressure { pascal: value }),
            other => {
                return Err(CliError::usage(format!(
                    "bme680 has no measurement `{other}`"
                )))
            }
        };
        Ok(Bme680 {
            measurement: Some(measurement),
        })
    }

    fn scd41_measurement(name: &str, value: f64) -> Result<Scd41, CliError> {
        use crate::protocol::generated::sensor::v1::scd41::Measurement;
        let measurement = match name {
            "co2" => Measurement::Co2(Co2 { ppm: value }),
            "temperature" => Measurement::Temperature(Temperature { celsius: value }),
            "humidity" => Measurement::Humidity(Humidity { percent: value }),
            other => {
                return Err(CliError::usage(format!(
                    "scd41 has no measurement `{other}`"
                )))
            }
        };
        Ok(Scd41 {
            measurement: Some(measurement),
        })
    }

    fn sds011_measurement(name: &str, value: f64) -> Result<Sds011, CliError> {
        use crate::protocol::generated::sensor::v1::sds011::Measurement;
        let measurement = match name {
            "pm25" => Measurement::Pm25(Pm25 { ug_m3: value }),
            "pm10" => Measurement::Pm10(Pm10 { ug_m3: value }),
            other => {
                return Err(CliError::usage(format!(
                    "sds011 has no measurement `{other}`"
                )))
            }
        };
        Ok(Sds011 {
            measurement: Some(measurement),
        })
    }

    fn ics43434_measurement(name: &str, value: f64) -> Result<Ics43434, CliError> {
        use crate::protocol::generated::sensor::v1::ics43434::Measurement;
        let measurement = match name {
            "noise_max" => Measurement::NoiseMax(NoiseLevel { db: value }),
            "noise_avg" => Measurement::NoiseAvg(NoiseLevel { db: value }),
            other => {
                return Err(CliError::usage(format!(
                    "ics43434 has no measurement `{other}`"
                )))
            }
        };
        Ok(Ics43434 {
            measurement: Some(measurement),
        })
    }

    /// Parse one non-empty, non-comment grammar line.
    fn parse_line(line: &str) -> Result<Line, String> {
        let (lhs, value_text) = line
            .split_once('=')
            .ok_or_else(|| "expected `<...>=<value>`".to_string())?;

        let (is_private, rest) = match lhs.split_once(':') {
            Some(("public", rest)) => (false, rest),
            Some(("private", rest)) => (true, rest),
            Some((other, _)) => return Err(format!("unknown section `{other}`")),
            None => (false, lhs),
        };

        let mut parts: Vec<&str> = rest.split('/').collect();
        let tag = parts.pop().ok_or_else(|| "empty line".to_string())?;

        let recipient = if is_private {
            if parts.is_empty() {
                return Err("`private:` requires a recipient".to_string());
            }
            Some(parts.remove(0).to_string())
        } else {
            None
        };

        let mut board_hint = None;
        if let Some(b) = parts.first() {
            board_hint = Some(
                Board::parse(b)
                    .ok_or_else(|| format!("unknown board `{b}` (want urban/insight)"))?,
            );
            parts.remove(0);
        }
        if !parts.is_empty() {
            return Err("too many `/`-separated segments".to_string());
        }

        // GPS positional special-case: bare `gps=<lat>,<lon>,<height_m>`.
        if tag == "gps" {
            let board = board_hint
                .ok_or_else(|| "`gps` requires an explicit board (urban/insight)".to_string())?;
            let coords: Vec<&str> = value_text.split(',').map(str::trim).collect();
            let (lat, lon, height_m) = match coords.as_slice() {
                [lat, lon] => (*lat, *lon, "0"),
                [lat, lon, height_m] => (*lat, *lon, *height_m),
                _ => return Err("`gps` expects `lat,lon[,height_m]`".to_string()),
            };
            let parse_f64 = |s: &str| {
                s.parse::<f64>()
                    .map_err(|e| format!("invalid gps coordinate `{s}`: {e}"))
            };
            return Ok(Line {
                recipient,
                board,
                kind: SensorKind::Gps,
                value: Value::Gps {
                    lat: parse_f64(lat)?,
                    lon: parse_f64(lon)?,
                    height_m: parse_f64(height_m)?,
                },
            });
        }

        let (sensor_hint, measurement) = match tag.split_once('.') {
            Some((s, m)) => (
                Some(SensorKind::parse(s).ok_or_else(|| format!("unknown sensor `{s}`"))?),
                m,
            ),
            None => (None, tag),
        };
        let measurement = canonical_measurement(measurement);
        let value: f64 = value_text
            .trim()
            .parse()
            .map_err(|e| format!("invalid value `{value_text}`: {e}"))?;

        let candidates: Vec<SensorKind> = SCALAR_SENSORS
            .iter()
            .copied()
            .filter(|k| sensor_hint.is_none_or(|h| h == *k))
            .filter(|k| board_hint.is_none_or(|b| k.board() == Some(b)))
            .filter(|k| k.measurements().contains(&measurement))
            .collect();

        let kind = match candidates.as_slice() {
            [k] => *k,
            [] => {
                return Err(format!(
                    "no sensor produces measurement `{measurement}` for the given board/sensor"
                ))
            }
            _ => {
                return Err(format!(
                    "ambiguous measurement `{measurement}`: specify a board or sensor"
                ))
            }
        };
        let board = kind
            .board()
            .or(board_hint)
            .ok_or_else(|| "ambiguous board: specify urban or insight".to_string())?;

        Ok(Line {
            recipient,
            board,
            kind,
            value: Value::Measurement {
                name: measurement_static(measurement),
                value,
            },
        })
    }

    /// Recover a `'static` measurement name from its canonical string (all
    /// canonical names are literals from [`SensorKind::measurements`]).
    fn measurement_static(name: &str) -> &'static str {
        match name {
            "temperature" => "temperature",
            "humidity" => "humidity",
            "pressure" => "pressure",
            "co2" => "co2",
            "pm25" => "pm25",
            "pm10" => "pm10",
            "noise_max" => "noise_max",
            "noise_avg" => "noise_avg",
            _ => unreachable!("validated against SensorKind::measurements above"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_urban() -> Message {
        Message {
            metadata: Some(Meta {
                owner: vec![0xabu8; 32],
            }),
            payload: Some(Payload::Urban(Urban {
                public: vec![
                    UrbanSensor {
                        sensor: Some(
                            crate::protocol::generated::device::v1::urban_sensor::Sensor::Gps(
                                Gps {
                                    lat: 55.75,
                                    lon: 37.61,
                                    height_m: 150.0,
                                },
                            ),
                        ),
                    },
                    UrbanSensor {
                        sensor: Some(
                            crate::protocol::generated::device::v1::urban_sensor::Sensor::Bme280(
                                Bme280 {
                                    measurement: Some(
                                        crate::protocol::generated::sensor::v1::bme280::Measurement::Temperature(
                                            Temperature { celsius: 21.5 },
                                        ),
                                    ),
                                },
                            ),
                        ),
                    },
                ],
                private: vec![],
            })),
        }
    }

    #[test]
    fn wire_roundtrip_via_protocol() {
        let msg = sample_urban();
        let wire = protocol::encode_sensor_message(&msg);
        let decoded = protocol::decode_sensor_message(&wire).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn grammar_public_measurement_with_alias() {
        let text = "bme280.temp=21.5\nurban/gps=55.75,37.61,150\n";
        let msg = grammar::parse(text, None).unwrap();
        match msg.payload.unwrap() {
            Payload::Urban(urban) => assert_eq!(urban.public.len(), 2),
            _ => panic!("expected urban payload"),
        }
    }

    #[test]
    fn grammar_infers_insight_board_from_measurement() {
        let msg = grammar::parse("co2=800\n", None).unwrap();
        match msg.payload.unwrap() {
            Payload::Insight(insight) => assert_eq!(insight.public.len(), 1),
            _ => panic!("expected insight payload"),
        }
    }

    #[test]
    fn grammar_rejects_mixed_boards() {
        let text = "urban/bme280.temp=21.5\ninsight/bme680.temp=22.0\n";
        let err = grammar::parse(text, None).unwrap_err();
        assert!(err.message.contains("same board"), "{}", err.message);
    }

    #[test]
    fn grammar_private_without_suri_is_usage_error() {
        let text =
            "private:5C4hrfjw9DjXZTzV3MwzrrAr9P1MJhSrvWGWqi1eSuyUpnhM/urban/bme280.temp=21.5\n";
        let err = grammar::parse(text, None).unwrap_err();
        assert_eq!(err.code, 2);
    }

    /// End-to-end: encrypt a private measurement for a recipient (via the
    /// line grammar), round-trip it through the wire encoding, then decrypt
    /// it back with [`decrypt_entry`] — mirroring what
    /// `edge message --decrypt --suri <SURI>` does.
    #[test]
    fn private_section_roundtrips_through_decrypt() {
        let sender = protocol::SensorIdentity::from_secret_bytes(&[7u8; 32]);
        let recipient = protocol::SensorIdentity::from_secret_bytes(&[9u8; 32]);
        let recipient_suri = recipient.secret_to_hex();
        let recipient_ss58 = recipient.sensor_id().to_ss58();

        let text = format!("private:{recipient_ss58}/bme280.temp=21.5\n");
        let msg = grammar::parse(&text, Some(&sender.secret_to_hex())).unwrap();

        // The private section is genuinely encrypted: no plaintext leaks
        // into the wire bytes.
        let wire = protocol::encode_sensor_message(&msg);
        let decoded = protocol::decode_sensor_message(&wire).unwrap();
        let entry = match &decoded.payload {
            Some(Payload::Urban(urban)) => {
                assert!(urban.public.is_empty());
                assert_eq!(urban.private.len(), 1);
                &urban.private[0]
            }
            _ => panic!("expected urban payload"),
        };

        // Decrypting with the intended recipient's SURI recovers the
        // measurement, decoded through the `EncryptedUrban` protobuf
        // declaration (the same one used to encrypt it).
        let decrypted = decrypt_entry::<EncryptedUrban>(entry, &recipient_suri).unwrap();
        assert_eq!(decrypted.sensors.len(), 1);
        match decrypted.sensors[0].sensor {
            Some(crate::protocol::generated::device::v1::urban_sensor::Sensor::Bme280(
                Bme280 {
                    measurement:
                        Some(crate::protocol::generated::sensor::v1::bme280::Measurement::Temperature(
                            Temperature { celsius },
                        )),
                },
            )) => assert_eq!(celsius, 21.5),
            _ => panic!("expected decrypted bme280 temperature"),
        }

        // Decrypting with an unrelated SURI must not recover the
        // measurement, and must not error the whole command either (the
        // failure is surfaced as a `Result::Err` for the caller to warn on).
        let stranger = protocol::SensorIdentity::from_secret_bytes(&[42u8; 32]);
        let err = decrypt_entry::<EncryptedUrban>(entry, &stranger.secret_to_hex()).unwrap_err();
        let _ = err;
    }
}
