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
//! compact measurements directly as argv values — the shell already
//! tokenizes for us, so there is no line/comment grammar to parse:
//!
//! ```sh
//! edge m -b urban temp=21.5 humidity=44 gps=51.5,-0.12,35
//! ```
//!
//! Each positional value is one measurement:
//!
//! ```text
//! [<sensor>.]<measurement>=<value>
//! private:<recipient>/[<sensor>.]<measurement>=<value>
//! ```
//!
//! - `--board`/`-b` (`urban` or `insight`) selects `Message.payload` for the
//!   whole command; it is required whenever measurement values are given.
//! - `<recipient>`: SS58 or `0x`-prefixed hex public key, only allowed after
//!   a `private:` prefix. Private measurements are grouped by recipient,
//!   serialized together, and encrypted once per recipient via `libcps`
//!   (`--suri` selects the sender identity).
//! - `<sensor>`: e.g. `bme280`, `bme680`, `scd41`, `sds011`, `ics43434`.
//!   Optional when the measurement is unambiguous on the selected board.
//! - `<measurement>`: the scalar field name (short aliases accepted, e.g.
//!   `temp` for `temperature`).
//! - GPS is a special case (three independent fields, not a oneof):
//!   `gps=<lat>,<lon>[,<height_m>]`.
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
    # Encode public measurements onto the `urban` board; wire bytes are
    # written to stdout as binary.
    edge message --board urban temp=21.5 humidity=44 > message.bin

    # Multiple public measurements across one message; short aliases (e.g.
    # `temp`) and the compact GPS syntax are both supported.
    edge m -b urban temp=21.5 humidity=44 gps=51.5,-0.12,35 --output hex

    # Ambiguous measurements on a board need an explicit sensor qualifier.
    edge m -b insight bme680.temp=21.5 scd41.co2=800

    # A private measurement, encrypted for its recipient with the sender's
    # key; private measurements for the same recipient are grouped and
    # encrypted together as a single ciphertext.
    edge m -b urban private:5FHneW46xGXgs5mUiveU4sbTyGBzmstUspZC92UhjJM694ty/temp=21.5 \\
      --suri 0xd6a1...seed... --output hex

    # Decode a wire message (hex here) back into a human-readable debug dump.
    edge message 0xdeadbeef.. --input hex --output text")]
pub(crate) struct MessageArgs {
    /// Compact measurements when encoding from argv (one per value, e.g.
    /// `temp=21.5` or `bme680.co2=800`), or a single encoded DATA value with
    /// an explicit `--input`. If no values are given, reads encoded data
    /// from stdin instead.
    #[arg(value_name = "MEASUREMENT")]
    values: Vec<String>,
    /// Device board for compact measurement encoding; required whenever
    /// measurement values are given.
    #[arg(short, long, value_enum)]
    board: Option<BoardArg>,
    /// Force the input representation of encoded DATA (otherwise sniffed:
    /// hex/base64/binary wire bytes, or JSON). Not used for compact
    /// measurement encoding.
    #[arg(short, long, value_enum)]
    input: Option<MessageFormat>,
    /// Force the output representation (otherwise: `text` when stdout is a
    /// terminal, `binary` otherwise).
    #[arg(short, long, value_enum)]
    output: Option<MessageFormat>,
    /// Identity used for encryption: sender when compact measurements
    /// contain `private:` entries (they are encrypted for their recipient),
    /// or recipient with `--decrypt`. A Substrate SURI (a `0x`-prefixed hex
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

/// Device board — selects `Message.payload`. Command-level state for
/// compact measurement encoding (see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum BoardArg {
    Urban,
    Insight,
}

impl From<BoardArg> for compact::Board {
    fn from(board: BoardArg) -> Self {
        match board {
            BoardArg::Urban => compact::Board::Urban,
            BoardArg::Insight => compact::Board::Insight,
        }
    }
}

/// Input/output representation for `edge message`. This only covers actual
/// data representations; compact measurements are CLI arguments, not an
/// input serialization format (see [`compact`]).
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
}

/// Dispatch `edge message`.
///
/// Three modes, selected structurally rather than by sniffing stdin bytes:
///
/// - one or more `values` with no `--input`: compact measurement argv mode
///   (requires `--board`);
/// - no `values`: read encoded/structured data from stdin (`--input`
///   forced or sniffed);
/// - one `values` entry with an explicit `--input`: treat it as encoded DATA
///   rather than a measurement (decode/convert convenience).
pub(crate) fn run(args: MessageArgs) -> CliResult {
    let wire = if args.input.is_none() && !args.values.is_empty() {
        let board = args.board.ok_or_else(|| {
            CliError::usage("`--board` is required to encode compact measurements")
        })?;
        let mut msg = compact::parse(board.into(), &args.values, args.suri.as_deref())?;
        apply_owner(&mut msg, args.owner.as_deref())?;
        protocol::encode_sensor_message(&msg)
    } else {
        let raw = match args.values.as_slice() {
            [] => format::read_stdin()?,
            [data] => data.clone().into_bytes(),
            _ => {
                return Err(CliError::usage(
                    "at most one encoded DATA value is accepted with `--input`",
                ))
            }
        };

        let input = args.input.unwrap_or_else(|| sniff_input(&raw));
        match input {
            MessageFormat::Binary => raw,
            MessageFormat::Base64 => format::wire_from_input(&raw, ByteFormat::Base64)?,
            MessageFormat::Hex => format::wire_from_input(&raw, ByteFormat::Hex)?,
            MessageFormat::Text => {
                return Err(CliError::usage(
                    "--input text is not supported; provide wire bytes (hex/base64/binary)",
                ))
            }
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
/// Applies whether the input was decoded (wire bytes) or just encoded
/// (compact measurements): a fresh interactive `edge message` shows what was
/// built.
fn default_output() -> MessageFormat {
    if std::io::stdout().is_terminal() {
        MessageFormat::Text
    } else {
        MessageFormat::Binary
    }
}

/// Sniff the [`MessageFormat`] of encoded `raw` data (never compact
/// measurements, which are dispatched separately by argv shape).
fn sniff_input(raw: &[u8]) -> MessageFormat {
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
            "private: from={} algorithm={}",
            SensorId::from_slice(&entry.from[..])
                .expect("entry.from is 32 byte lenght")
                .to_ss58(),
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
/// declared alongside the encoder in [`compact::encrypt_group`].
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
// Compact argv syntax
// ---------------------------------------------------------------------------

mod compact {
    use super::*;

    /// Device board — selects `Message.payload`. Command-level state: known
    /// before any measurement is parsed (see the module docs).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Board {
        Urban,
        Insight,
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
                "bme280" => Some(Self::Bme280),
                "bme680" => Some(Self::Bme680),
                "scd41" => Some(Self::Scd41),
                "sds011" => Some(Self::Sds011),
                "ics43434" => Some(Self::Ics43434),
                _ => None,
            }
        }

        /// The board this sensor belongs to (fixed; `Gps` is valid on both
        /// boards and never reaches this method).
        fn board(self) -> Board {
            match self {
                Self::Bme280 | Self::Sds011 | Self::Ics43434 => Board::Urban,
                Self::Bme680 | Self::Scd41 => Board::Insight,
                Self::Gps => unreachable!("Gps has no fixed board"),
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

    /// A single resolved value carried by a compact measurement item.
    enum Value {
        Measurement { name: &'static str, value: f64 },
        Gps { lat: f64, lon: f64, height_m: f64 },
    }

    /// One fully-resolved compact measurement item.
    struct Item {
        recipient: Option<String>,
        kind: SensorKind,
        value: Value,
    }

    /// Parse `values` (one compact measurement item per argv value) for
    /// `board` into a `core.v1.Message`.
    ///
    /// `suri` is required (and used) only if `values` contains any
    /// `private:` items.
    pub(super) fn parse(
        board: Board,
        values: &[String],
        suri: Option<&str>,
    ) -> Result<Message, CliError> {
        if values.is_empty() {
            return Err(CliError::usage("no measurements provided"));
        }

        let mut public_urban = Vec::new();
        let mut public_insight = Vec::new();
        // Recipient groups, keyed by recipient string, preserving first-seen order.
        let mut private_urban: Vec<(String, Vec<UrbanSensor>)> = Vec::new();
        let mut private_insight: Vec<(String, Vec<InsightSensor>)> = Vec::new();

        for raw in values {
            let item = parse_item(board, raw)
                .map_err(|e| CliError::usage(format!("invalid measurement `{raw}`: {e}")))?;

            match (item.recipient, board) {
                (None, Board::Urban) => public_urban.push(build_urban(item.kind, &item.value)?),
                (None, Board::Insight) => {
                    public_insight.push(build_insight(item.kind, &item.value)?)
                }
                (Some(recipient), Board::Urban) => {
                    let entry = build_urban(item.kind, &item.value)?;
                    group_push(&mut private_urban, recipient, entry);
                }
                (Some(recipient), Board::Insight) => {
                    let entry = build_insight(item.kind, &item.value)?;
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

    /// Parse one compact measurement argv item:
    ///
    /// ```text
    /// [<sensor>.]<measurement>=<value>
    /// private:<recipient>/[<sensor>.]<measurement>=<value>
    /// gps=<lat>,<lon>[,<height_m>]
    /// private:<recipient>/gps=<lat>,<lon>[,<height_m>]
    /// ```
    fn parse_item(board: Board, raw: &str) -> Result<Item, String> {
        let (lhs, value_text) = raw
            .split_once('=')
            .ok_or_else(|| "expected `<measurement>=<value>`".to_string())?;

        let (recipient, tag) = if let Some(rest) = lhs.strip_prefix("private:") {
            let (recipient, tag) = rest
                .split_once('/')
                .ok_or_else(|| "`private:` requires `<recipient>/<measurement>`".to_string())?;
            (Some(recipient.to_string()), tag)
        } else {
            (None, lhs)
        };

        // GPS positional special-case: bare `gps=<lat>,<lon>[,<height_m>]`.
        if tag == "gps" {
            let mut coords = value_text.split(',').map(str::trim);
            let lat = coords
                .next()
                .ok_or_else(|| "`gps` expects `lat,lon[,height_m]`".to_string())?;
            let lon = coords
                .next()
                .ok_or_else(|| "`gps` expects `lat,lon[,height_m]`".to_string())?;
            let height_m = coords.next().unwrap_or("0");
            if coords.next().is_some() {
                return Err("`gps` expects `lat,lon[,height_m]`".to_string());
            }
            let parse_f64 = |s: &str| {
                s.parse::<f64>()
                    .map_err(|e| format!("invalid gps coordinate `{s}`: {e}"))
            };
            return Ok(Item {
                recipient,
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

        if let Some(hint) = sensor_hint {
            if hint.board() != board {
                return Err(format!(
                    "sensor `{}` does not belong to the {} board",
                    tag.split_once('.').map(|(s, _)| s).unwrap_or(""),
                    board_name(board)
                ));
            }
        }

        let candidates: Vec<SensorKind> = SCALAR_SENSORS
            .iter()
            .copied()
            .filter(|k| k.board() == board)
            .filter(|k| sensor_hint.is_none_or(|h| h == *k))
            .filter(|k| k.measurements().contains(&measurement))
            .collect();

        let kind = match candidates.as_slice() {
            [k] => *k,
            [] => {
                return Err(format!(
                    "no sensor produces measurement `{measurement}` on the {} board",
                    board_name(board)
                ))
            }
            many => {
                let sensors = many
                    .iter()
                    .map(|k| format!("`{}.{measurement}`", sensor_name(*k)))
                    .collect::<Vec<_>>()
                    .join(" or ");
                return Err(format!(
                    "ambiguous measurement `{measurement}` on {} board; specify {sensors}",
                    board_name(board)
                ));
            }
        };

        Ok(Item {
            recipient,
            kind,
            value: Value::Measurement {
                name: measurement_static(measurement),
                value,
            },
        })
    }

    fn board_name(board: Board) -> &'static str {
        match board {
            Board::Urban => "urban",
            Board::Insight => "insight",
        }
    }

    fn sensor_name(kind: SensorKind) -> &'static str {
        match kind {
            SensorKind::Gps => "gps",
            SensorKind::Bme280 => "bme280",
            SensorKind::Bme680 => "bme680",
            SensorKind::Scd41 => "scd41",
            SensorKind::Sds011 => "sds011",
            SensorKind::Ics43434 => "ics43434",
        }
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

    /// Helper: build the `values` slice `compact::parse` expects from plain
    /// string literals.
    fn values(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn compact_urban_measurement_with_alias_and_gps() {
        let msg = compact::parse(
            compact::Board::Urban,
            &values(&["bme280.temp=21.5", "gps=55.75,37.61,150"]),
            None,
        )
        .unwrap();
        match msg.payload.unwrap() {
            Payload::Urban(urban) => assert_eq!(urban.public.len(), 2),
            _ => panic!("expected urban payload"),
        }
    }

    #[test]
    fn compact_urban_unambiguous_temp_resolves_to_bme280() {
        let msg = compact::parse(compact::Board::Urban, &values(&["temp=20"]), None).unwrap();
        match msg.payload.unwrap() {
            Payload::Urban(urban) => {
                assert_eq!(urban.public.len(), 1);
                match urban.public[0].sensor {
                    Some(crate::protocol::generated::device::v1::urban_sensor::Sensor::Bme280(
                        _,
                    )) => {}
                    _ => panic!("expected bme280"),
                }
            }
            _ => panic!("expected urban payload"),
        }
    }

    #[test]
    fn compact_urban_pm25_resolves_to_sds011() {
        let msg = compact::parse(compact::Board::Urban, &values(&["pm25=12"]), None).unwrap();
        match msg.payload.unwrap() {
            Payload::Urban(urban) => assert_eq!(urban.public.len(), 1),
            _ => panic!("expected urban payload"),
        }
    }

    #[test]
    fn compact_insight_co2_resolves_to_scd41() {
        let msg = compact::parse(compact::Board::Insight, &values(&["co2=800"]), None).unwrap();
        match msg.payload.unwrap() {
            Payload::Insight(insight) => assert_eq!(insight.public.len(), 1),
            _ => panic!("expected insight payload"),
        }
    }

    #[test]
    fn compact_insight_ambiguous_temp_requires_sensor_qualifier() {
        let err =
            compact::parse(compact::Board::Insight, &values(&["temp=21.5"]), None).unwrap_err();
        assert!(
            err.message.contains("ambiguous measurement"),
            "{}",
            err.message
        );
        assert!(err.message.contains("bme680.temp"), "{}", err.message);
        assert!(err.message.contains("scd41.temp"), "{}", err.message);
    }

    #[test]
    fn compact_insight_qualified_sensors_disambiguate() {
        let msg = compact::parse(
            compact::Board::Insight,
            &values(&["bme680.temp=21.5", "scd41.co2=800"]),
            None,
        )
        .unwrap();
        match msg.payload.unwrap() {
            Payload::Insight(insight) => assert_eq!(insight.public.len(), 2),
            _ => panic!("expected insight payload"),
        }
    }

    #[test]
    fn compact_sensor_incompatible_with_board_is_rejected() {
        let err =
            compact::parse(compact::Board::Urban, &values(&["scd41.co2=800"]), None).unwrap_err();
        assert!(err.message.contains("does not belong"), "{}", err.message);
    }

    #[test]
    fn compact_private_without_suri_is_usage_error() {
        let err = compact::parse(
            compact::Board::Urban,
            &values(&["private:5C4hrfjw9DjXZTzV3MwzrrAr9P1MJhSrvWGWqi1eSuyUpnhM/bme280.temp=21.5"]),
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, 2);
    }

    /// End-to-end: encrypt a private measurement for a recipient (via the
    /// compact argv syntax), round-trip it through the wire encoding, then
    /// decrypt it back with [`decrypt_entry`] — mirroring what
    /// `edge message --decrypt --suri <SURI>` does.
    #[test]
    fn private_section_roundtrips_through_decrypt() {
        let sender = protocol::SensorIdentity::from_secret_bytes(&[7u8; 32]);
        let recipient = protocol::SensorIdentity::from_secret_bytes(&[9u8; 32]);
        let recipient_suri = recipient.secret_to_hex();
        let recipient_ss58 = recipient.sensor_id().to_ss58();

        let item = format!("private:{recipient_ss58}/bme280.temp=21.5");
        let msg = compact::parse(
            compact::Board::Urban,
            &values(&[&item]),
            Some(&sender.secret_to_hex()),
        )
        .unwrap();

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
