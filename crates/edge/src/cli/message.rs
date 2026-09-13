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
//! separate `encode`/`decode` subcommands. On top of the structured JSON
//! representation, encode also accepts a compact line-oriented grammar (one
//! measurement per line) so telemetry can be hand-written or scripted without
//! writing JSON:
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

use super::format::{self, ByteFormat, Direction};
use super::{CliError, CliResult};
use crate::protocol::sensor_message::{Message, Meta, Payload};
use crate::protocol::{
    self, Bme280, Bme680, Co2, Encrypted, EncryptedInsight, EncryptedUrban, Gps, Humidity,
    Ics43434, Insight, InsightSensor, NoiseLevel, Pm10, Pm25, Pressure, Scd41, Sds011, SensorId,
    Temperature, Urban, UrbanSensor,
};
use clap::{Args, ValueEnum};
use prost::Message as _;
use serde::{Deserialize, Serialize};
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

    # Decode a wire message (hex here) back into structured JSON.
    edge message <0xdeadbeef.. --input hex --output json")]
pub(crate) struct MessageArgs {
    /// Message data. If omitted, reads from stdin. Decoded per `--input` (or
    /// sniffed: hex/base64/binary wire bytes).
    data: Option<String>,
    /// Force the input representation (otherwise sniffed: JSON encodes,
    /// line-grammar text encodes, anything else decodes).
    #[arg(short, long, value_enum)]
    input: Option<MessageFormat>,
    /// Force the output representation (otherwise: `json` when decoding,
    /// `binary` when encoding).
    #[arg(short, long, value_enum)]
    output: Option<MessageFormat>,
    /// Identity used for encryption: sender when the line grammar contains
    /// `private:` entries (they are encrypted for their recipient), or
    /// recipient with `--decrypt`. A Substrate SURI (a `0x`-prefixed hex
    /// seed or a BIP-39 phrase, with optional derivation junctions).
    #[arg(short, long, value_name = "SURI")]
    suri: Option<String>,
    /// Sensor owner public key (SS58 or `0x`-prefixed hex) to set on the
    /// encoded message's `metadata.owner` field. Overrides any `owner`
    /// present in JSON input. Unset (no owner) by default.
    #[arg(long, value_name = "ADDRESS")]
    owner: Option<String>,
    /// Print the message's dedup id (SHA-256 of the encoded wire bytes) to
    /// stderr as `message_id: <hex>`. Diagnostics only; stdout stays a clean
    /// data stream.
    #[arg(long)]
    id: bool,
    /// Decrypt `private` measurement sections when decoding to JSON, using
    /// `--suri` as the recipient's private key (the sender's public key
    /// travels with each entry, so it isn't needed separately). Each entry
    /// is decrypted independently and added as `decrypted` alongside its
    /// `ciphertext`; entries this key can't open (e.g. addressed to someone
    /// else) are left ciphertext-only, with a warning on stderr. No effect
    /// on non-JSON output or when encoding.
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
    /// Structured JSON.
    Json,
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
        MessageFormat::Json => {
            let dto: MessageJson = serde_json::from_slice(&raw)
                .map_err(|e| CliError::with_code(3, format!("invalid JSON: {e}")))?;
            let mut msg = dto.into_proto()?;
            apply_owner(&mut msg, args.owner.as_deref())?;
            protocol::encode_sensor_message(&msg)
        }
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
    };

    let output = args.output.unwrap_or(match input {
        MessageFormat::Json | MessageFormat::Grammar => MessageFormat::Binary,
        MessageFormat::Binary | MessageFormat::Base64 | MessageFormat::Hex => MessageFormat::Json,
    });

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

/// Sniff the [`MessageFormat`] of `raw`: JSON if it parses as one, else the
/// line grammar if it looks like `key=value` text, else wire bytes.
fn sniff_input(raw: &[u8]) -> MessageFormat {
    if format::sniff_direction(raw) == Direction::Encode {
        return MessageFormat::Json;
    }
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
/// measurement sections when writing JSON (see [`MessageJson::from_proto`]).
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
        MessageFormat::Json => {
            let msg = protocol::decode_sensor_message(wire).map_err(format::protocol_err)?;
            let dto = MessageJson::from_proto(&msg, decrypt_suri)?;
            let json = serde_json::to_string_pretty(&dto)
                .map_err(|e| CliError::runtime(format!("failed to serialize JSON: {e}")))?;
            println!("{json}");
            Ok(())
        }
        MessageFormat::Grammar => Err(CliError::usage(
            "`--output grammar` is not supported: the line grammar is encode-only",
        )),
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

// ---------------------------------------------------------------------------
// JSON DTO
// ---------------------------------------------------------------------------

/// JSON data-transfer object for a `core.v1.Message`.
///
/// Byte fields are `0x`-prefixed hex, matching the `envelope` JSON DTO
/// convention.
#[derive(Debug, Serialize, Deserialize)]
struct MessageJson {
    /// Sensor owner public key (`0x`-prefixed hex). Empty string if not set.
    #[serde(default)]
    owner: String,
    /// Device payload: exactly one of `urban` or `insight` must be present.
    #[serde(flatten)]
    device: DeviceJson,
}

/// The `Message.payload` oneof: outdoor (`urban`) or indoor (`insight`) boards.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DeviceJson {
    Urban(BoardJson<UrbanSensorJson>),
    Insight(BoardJson<InsightSensorJson>),
}

/// Public/private measurement sections shared by `Urban` and `Insight`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(deserialize = "S: Deserialize<'de>"))]
struct BoardJson<S> {
    /// Public measurements, visible to everyone.
    #[serde(default)]
    public: Vec<S>,
    /// Encrypted measurement sections, one per recipient. Each entry carries
    /// its `decrypted` sensors (populated only with `--decrypt`, and only
    /// for entries that decrypt successfully with the given `--suri`).
    #[serde(default)]
    private: Vec<EncryptedJson<S>>,
}

/// JSON form of `crypto.v1.Encrypted`. This codec never decrypts
/// `ciphertext` on its own; `decrypted` is populated separately (see
/// [`BoardJson`]) when `--decrypt` succeeds for this entry.
#[derive(Debug, Serialize, Deserialize)]
#[serde(bound(deserialize = "S: Deserialize<'de>"))]
struct EncryptedJson<S> {
    version: u32,
    algorithm: String,
    from: String,
    nonce: String,
    ciphertext: String,
    /// Decrypted sensors for this entry, if `--decrypt` was given, a
    /// matching `--suri` was supplied, and decryption succeeded. Omitted
    /// (and ignored on input) otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decrypted: Option<Vec<S>>,
}

impl<S> EncryptedJson<S> {
    fn from_proto(e: &Encrypted) -> Self {
        Self {
            version: e.version,
            algorithm: e.algorithm.clone(),
            from: format!("0x{}", hex::encode(&e.from)),
            nonce: format!("0x{}", hex::encode(&e.nonce)),
            ciphertext: format!("0x{}", hex::encode(&e.ciphertext)),
            decrypted: None,
        }
    }

    fn into_proto(self) -> Result<Encrypted, CliError> {
        Ok(Encrypted {
            version: self.version,
            algorithm: self.algorithm,
            from: decode_hex("private[].from", &self.from)?,
            nonce: decode_hex("private[].nonce", &self.nonce)?,
            ciphertext: decode_hex("private[].ciphertext", &self.ciphertext)?,
        })
    }
}

/// Build an urban `private` entry's JSON, attempting decryption with
/// `decrypt_suri` (the recipient's private key) if given. The sender's
/// public key travels with the entry itself (`from`), so only the
/// recipient's identity is needed here. Decryption failures (e.g. this
/// `--suri` isn't the intended recipient) are reported on stderr and leave
/// the entry as ciphertext rather than aborting the whole command.
fn encrypted_urban_json(
    entry: &Encrypted,
    decrypt_suri: Option<&str>,
) -> EncryptedJson<UrbanSensorJson> {
    let mut dto = EncryptedJson::from_proto(entry);
    if let Some(suri) = decrypt_suri {
        match decrypt_entry::<EncryptedUrban>(entry, suri) {
            Ok(decoded) => {
                dto.decrypted = Some(
                    decoded
                        .sensors
                        .iter()
                        .map(UrbanSensorJson::from_proto)
                        .collect::<Result<_, _>>()
                        .unwrap_or_else(|e| {
                            eprintln!(
                                "warning: decrypted urban entry has invalid sensor data: {e:?}"
                            );
                            Vec::new()
                        }),
                );
            }
            Err(e) => eprintln!(
                "warning: failed to decrypt private entry from 0x{}: {e:?}",
                hex::encode(&entry.from)
            ),
        }
    }
    dto
}

/// Insight counterpart of [`encrypted_urban_json`].
fn encrypted_insight_json(
    entry: &Encrypted,
    decrypt_suri: Option<&str>,
) -> EncryptedJson<InsightSensorJson> {
    let mut dto = EncryptedJson::from_proto(entry);
    if let Some(suri) = decrypt_suri {
        match decrypt_entry::<EncryptedInsight>(entry, suri) {
            Ok(decoded) => {
                dto.decrypted = Some(
                    decoded
                        .sensors
                        .iter()
                        .map(InsightSensorJson::from_proto)
                        .collect::<Result<_, _>>()
                        .unwrap_or_else(|e| {
                            eprintln!(
                                "warning: decrypted insight entry has invalid sensor data: {e:?}"
                            );
                            Vec::new()
                        }),
                );
            }
            Err(e) => eprintln!(
                "warning: failed to decrypt private entry from 0x{}: {e:?}",
                hex::encode(&entry.from)
            ),
        }
    }
    dto
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

/// One `sensor.v1` measurement wrapped by its unit field name, e.g.
/// `{"temperature": 21.5}`.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MeasurementJson {
    Temperature(f64),
    Humidity(f64),
    Pressure(f64),
    Co2(f64),
    Pm25(f64),
    Pm10(f64),
    NoiseMax(f64),
    NoiseAvg(f64),
}

/// `sensor.v1.GPS` JSON mirror.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct GpsJson {
    lat: f64,
    lon: f64,
    #[serde(default)]
    height_m: f64,
}

impl GpsJson {
    fn from_proto(g: &Gps) -> Self {
        Self {
            lat: g.lat,
            lon: g.lon,
            height_m: g.height_m,
        }
    }

    fn into_proto(self) -> Gps {
        Gps {
            lat: self.lat,
            lon: self.lon,
            height_m: self.height_m,
        }
    }
}

/// `device.v1.UrbanSensor` JSON mirror: one outdoor board reading per event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum UrbanSensorJson {
    Gps(GpsJson),
    Bme280(MeasurementJson),
    Sds011(MeasurementJson),
    Ics43434(MeasurementJson),
}

/// `device.v1.InsightSensor` JSON mirror: one indoor board reading per event.
#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum InsightSensorJson {
    Gps(GpsJson),
    Bme680(MeasurementJson),
    Scd41(MeasurementJson),
}

/// Decode a hex field, mapping failures to a stable `CliError` (exit code
/// `3`). The value must carry the mandatory `0x` prefix.
fn decode_hex(field: &str, value: &str) -> Result<Vec<u8>, CliError> {
    let stripped = crate::protocol::strip_0x(value.trim())
        .map_err(|e| CliError::with_code(3, format!("invalid hex in `{field}`: {e}")))?;
    hex::decode(stripped)
        .map_err(|e| CliError::with_code(3, format!("invalid hex in `{field}`: {e}")))
}

impl MessageJson {
    /// Build the JSON DTO from a decoded `core.v1.Message`. If `decrypt_suri`
    /// is set, attempts to decrypt each `private` entry using it as the
    /// recipient's private key (the sender's public key travels with the
    /// entry itself); entries that fail to decrypt with this key are left
    /// as ciphertext, with a warning on stderr.
    fn from_proto(msg: &Message, decrypt_suri: Option<&str>) -> Result<Self, CliError> {
        let owner = msg
            .metadata
            .as_ref()
            .map(|m| format!("0x{}", hex::encode(&m.owner)))
            .unwrap_or_default();
        let payload = msg
            .payload
            .as_ref()
            .ok_or_else(|| CliError::with_code(3, "message has no `payload` (urban/insight)"))?;
        let device = match payload {
            Payload::Urban(urban) => DeviceJson::Urban(BoardJson {
                public: urban
                    .public
                    .iter()
                    .map(UrbanSensorJson::from_proto)
                    .collect::<Result<_, _>>()?,
                private: urban
                    .private
                    .iter()
                    .map(|e| encrypted_urban_json(e, decrypt_suri))
                    .collect(),
            }),
            Payload::Insight(insight) => DeviceJson::Insight(BoardJson {
                public: insight
                    .public
                    .iter()
                    .map(InsightSensorJson::from_proto)
                    .collect::<Result<_, _>>()?,
                private: insight
                    .private
                    .iter()
                    .map(|e| encrypted_insight_json(e, decrypt_suri))
                    .collect(),
            }),
        };
        Ok(Self { owner, device })
    }

    /// Convert the JSON DTO into a `core.v1.Message`, hex-decoding byte fields.
    fn into_proto(self) -> Result<Message, CliError> {
        let metadata = if self.owner.is_empty() {
            None
        } else {
            Some(Meta {
                owner: decode_hex("owner", &self.owner)?,
            })
        };
        let payload = match self.device {
            DeviceJson::Urban(board) => Payload::Urban(Urban {
                public: board
                    .public
                    .into_iter()
                    .map(UrbanSensorJson::into_proto)
                    .collect::<Result<_, _>>()?,
                private: board
                    .private
                    .into_iter()
                    .map(EncryptedJson::into_proto)
                    .collect::<Result<_, _>>()?,
            }),
            DeviceJson::Insight(board) => Payload::Insight(Insight {
                public: board
                    .public
                    .into_iter()
                    .map(InsightSensorJson::into_proto)
                    .collect::<Result<_, _>>()?,
                private: board
                    .private
                    .into_iter()
                    .map(EncryptedJson::into_proto)
                    .collect::<Result<_, _>>()?,
            }),
        };
        Ok(Message {
            metadata,
            payload: Some(payload),
        })
    }
}

impl UrbanSensorJson {
    fn from_proto(s: &UrbanSensor) -> Result<Self, CliError> {
        use crate::protocol::generated::device::v1::urban_sensor::Sensor;
        let sensor = s
            .sensor
            .as_ref()
            .ok_or_else(|| CliError::with_code(3, "urban sensor entry has no reading set"))?;
        Ok(match sensor {
            Sensor::Gps(g) => Self::Gps(GpsJson::from_proto(g)),
            Sensor::Bme280(b) => Self::Bme280(measurement_from_bme280(b)?),
            Sensor::Sds011(s) => Self::Sds011(measurement_from_sds011(s)?),
            Sensor::Ics43434(i) => Self::Ics43434(measurement_from_ics43434(i)?),
        })
    }

    fn into_proto(self) -> Result<UrbanSensor, CliError> {
        use crate::protocol::generated::device::v1::urban_sensor::Sensor;
        let sensor = match self {
            Self::Gps(g) => Sensor::Gps(g.into_proto()),
            Self::Bme280(m) => Sensor::Bme280(bme280_from_measurement(m)?),
            Self::Sds011(m) => Sensor::Sds011(sds011_from_measurement(m)?),
            Self::Ics43434(m) => Sensor::Ics43434(ics43434_from_measurement(m)?),
        };
        Ok(UrbanSensor {
            sensor: Some(sensor),
        })
    }
}

impl InsightSensorJson {
    fn from_proto(s: &InsightSensor) -> Result<Self, CliError> {
        use crate::protocol::generated::device::v1::insight_sensor::Sensor;
        let sensor = s
            .sensor
            .as_ref()
            .ok_or_else(|| CliError::with_code(3, "insight sensor entry has no reading set"))?;
        Ok(match sensor {
            Sensor::Gps(g) => Self::Gps(GpsJson::from_proto(g)),
            Sensor::Bme680(b) => Self::Bme680(measurement_from_bme680(b)?),
            Sensor::Scd41(s) => Self::Scd41(measurement_from_scd41(s)?),
        })
    }

    fn into_proto(self) -> Result<InsightSensor, CliError> {
        use crate::protocol::generated::device::v1::insight_sensor::Sensor;
        let sensor = match self {
            Self::Gps(g) => Sensor::Gps(g.into_proto()),
            Self::Bme680(m) => Sensor::Bme680(bme680_from_measurement(m)?),
            Self::Scd41(m) => Sensor::Scd41(scd41_from_measurement(m)?),
        };
        Ok(InsightSensor {
            sensor: Some(sensor),
        })
    }
}

/// Error raised when a sensor's `oneof measurement` field is unset.
fn missing_measurement(sensor: &str) -> CliError {
    CliError::with_code(3, format!("{sensor} entry has no measurement set"))
}

/// Error raised when a JSON measurement variant doesn't apply to `sensor`.
fn unsupported_measurement(sensor: &str) -> CliError {
    CliError::with_code(
        2,
        format!("`{sensor}` does not support this measurement kind"),
    )
}

fn measurement_from_bme280(b: &Bme280) -> Result<MeasurementJson, CliError> {
    use crate::protocol::generated::sensor::v1::bme280::Measurement;
    match b
        .measurement
        .as_ref()
        .ok_or_else(|| missing_measurement("bme280"))?
    {
        Measurement::Temperature(t) => Ok(MeasurementJson::Temperature(t.celsius)),
        Measurement::Humidity(h) => Ok(MeasurementJson::Humidity(h.percent)),
        Measurement::Pressure(p) => Ok(MeasurementJson::Pressure(p.pascal)),
    }
}

fn bme280_from_measurement(m: MeasurementJson) -> Result<Bme280, CliError> {
    use crate::protocol::generated::sensor::v1::bme280::Measurement;
    let measurement = match m {
        MeasurementJson::Temperature(v) => Measurement::Temperature(Temperature { celsius: v }),
        MeasurementJson::Humidity(v) => Measurement::Humidity(Humidity { percent: v }),
        MeasurementJson::Pressure(v) => Measurement::Pressure(Pressure { pascal: v }),
        _ => return Err(unsupported_measurement("bme280")),
    };
    Ok(Bme280 {
        measurement: Some(measurement),
    })
}

fn measurement_from_bme680(b: &Bme680) -> Result<MeasurementJson, CliError> {
    use crate::protocol::generated::sensor::v1::bme680::Measurement;
    match b
        .measurement
        .as_ref()
        .ok_or_else(|| missing_measurement("bme680"))?
    {
        Measurement::Temperature(t) => Ok(MeasurementJson::Temperature(t.celsius)),
        Measurement::Humidity(h) => Ok(MeasurementJson::Humidity(h.percent)),
        Measurement::Pressure(p) => Ok(MeasurementJson::Pressure(p.pascal)),
    }
}

fn bme680_from_measurement(m: MeasurementJson) -> Result<Bme680, CliError> {
    use crate::protocol::generated::sensor::v1::bme680::Measurement;
    let measurement = match m {
        MeasurementJson::Temperature(v) => Measurement::Temperature(Temperature { celsius: v }),
        MeasurementJson::Humidity(v) => Measurement::Humidity(Humidity { percent: v }),
        MeasurementJson::Pressure(v) => Measurement::Pressure(Pressure { pascal: v }),
        _ => return Err(unsupported_measurement("bme680")),
    };
    Ok(Bme680 {
        measurement: Some(measurement),
    })
}

fn measurement_from_scd41(s: &Scd41) -> Result<MeasurementJson, CliError> {
    use crate::protocol::generated::sensor::v1::scd41::Measurement;
    match s
        .measurement
        .as_ref()
        .ok_or_else(|| missing_measurement("scd41"))?
    {
        Measurement::Co2(c) => Ok(MeasurementJson::Co2(c.ppm)),
        Measurement::Temperature(t) => Ok(MeasurementJson::Temperature(t.celsius)),
        Measurement::Humidity(h) => Ok(MeasurementJson::Humidity(h.percent)),
    }
}

fn scd41_from_measurement(m: MeasurementJson) -> Result<Scd41, CliError> {
    use crate::protocol::generated::sensor::v1::scd41::Measurement;
    let measurement = match m {
        MeasurementJson::Co2(v) => Measurement::Co2(Co2 { ppm: v }),
        MeasurementJson::Temperature(v) => Measurement::Temperature(Temperature { celsius: v }),
        MeasurementJson::Humidity(v) => Measurement::Humidity(Humidity { percent: v }),
        _ => return Err(unsupported_measurement("scd41")),
    };
    Ok(Scd41 {
        measurement: Some(measurement),
    })
}

fn measurement_from_sds011(s: &Sds011) -> Result<MeasurementJson, CliError> {
    use crate::protocol::generated::sensor::v1::sds011::Measurement;
    match s
        .measurement
        .as_ref()
        .ok_or_else(|| missing_measurement("sds011"))?
    {
        Measurement::Pm25(p) => Ok(MeasurementJson::Pm25(p.ug_m3)),
        Measurement::Pm10(p) => Ok(MeasurementJson::Pm10(p.ug_m3)),
    }
}

fn sds011_from_measurement(m: MeasurementJson) -> Result<Sds011, CliError> {
    use crate::protocol::generated::sensor::v1::sds011::Measurement;
    let measurement = match m {
        MeasurementJson::Pm25(v) => Measurement::Pm25(Pm25 { ug_m3: v }),
        MeasurementJson::Pm10(v) => Measurement::Pm10(Pm10 { ug_m3: v }),
        _ => return Err(unsupported_measurement("sds011")),
    };
    Ok(Sds011 {
        measurement: Some(measurement),
    })
}

fn measurement_from_ics43434(i: &Ics43434) -> Result<MeasurementJson, CliError> {
    use crate::protocol::generated::sensor::v1::ics43434::Measurement;
    match i
        .measurement
        .as_ref()
        .ok_or_else(|| missing_measurement("ics43434"))?
    {
        Measurement::NoiseMax(n) => Ok(MeasurementJson::NoiseMax(n.db)),
        Measurement::NoiseAvg(n) => Ok(MeasurementJson::NoiseAvg(n.db)),
    }
}

fn ics43434_from_measurement(m: MeasurementJson) -> Result<Ics43434, CliError> {
    use crate::protocol::generated::sensor::v1::ics43434::Measurement;
    let measurement = match m {
        MeasurementJson::NoiseMax(v) => Measurement::NoiseMax(NoiseLevel { db: v }),
        MeasurementJson::NoiseAvg(v) => Measurement::NoiseAvg(NoiseLevel { db: v }),
        _ => return Err(unsupported_measurement("ics43434")),
    };
    Ok(Ics43434 {
        measurement: Some(measurement),
    })
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
    fn urban_message_json_roundtrip() {
        let msg = sample_urban();
        let json = MessageJson::from_proto(&msg, None).unwrap();
        let text = serde_json::to_string_pretty(&json).unwrap();
        let parsed: MessageJson = serde_json::from_str(&text).unwrap();
        let back = parsed.into_proto().unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn wire_roundtrip_via_protocol() {
        let msg = sample_urban();
        let wire = protocol::encode_sensor_message(&msg);
        let decoded = protocol::decode_sensor_message(&wire).unwrap();
        assert_eq!(msg, decoded);
    }

    #[test]
    fn missing_payload_is_reported() {
        let msg = Message {
            metadata: None,
            payload: None,
        };
        let err = MessageJson::from_proto(&msg, None).unwrap_err();
        assert!(err.message.contains("payload"));
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
    /// it back with `MessageJson::from_proto`'s `decrypt_suri` — mirroring
    /// what `edge message --decrypt --suri <SURI>` does.
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
        match &decoded.payload {
            Some(Payload::Urban(urban)) => {
                assert!(urban.public.is_empty());
                assert_eq!(urban.private.len(), 1);
            }
            _ => panic!("expected urban payload"),
        }

        // Decrypting with the intended recipient's SURI recovers the
        // measurement, decoded through the `EncryptedUrban` protobuf
        // declaration (the same one used to encrypt it).
        let json = MessageJson::from_proto(&decoded, Some(&recipient_suri)).unwrap();
        let DeviceJson::Urban(board) = &json.device else {
            panic!("expected urban device")
        };
        assert_eq!(board.private.len(), 1);
        let decrypted = board.private[0]
            .decrypted
            .as_ref()
            .expect("entry should decrypt with the recipient's suri");
        assert_eq!(
            decrypted,
            &vec![UrbanSensorJson::Bme280(MeasurementJson::Temperature(21.5))]
        );

        // Decrypting with an unrelated SURI must not recover the
        // measurement, and must not error the whole command either.
        let stranger = protocol::SensorIdentity::from_secret_bytes(&[42u8; 32]);
        let json = MessageJson::from_proto(&decoded, Some(&stranger.secret_to_hex())).unwrap();
        let DeviceJson::Urban(board) = &json.device else {
            panic!("expected urban device")
        };
        assert!(board.private[0].decrypted.is_none());
    }
}
