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
//! `edge sensor` (alias `s`) — hardware/integration senders that emit an
//! already-constructed `SignedEnvelope` onto a real transport.
//!
//! `edge sensor` deliberately does *not* build or sign telemetry itself; the
//! normal pipeline composes it with the existing commands:
//!
//! ```sh
//! edge message -b urban temp=21.5 humidity=44 \
//!   | edge envelope --sign "$SURI" \
//!   | edge sensor meshtastic ...
//! ```
//!
//! ## `edge sensor meshtastic`
//!
//! Reads one serialized `SignedEnvelope` from stdin (raw protobuf binary,
//! exact bytes preserved), fragments it per the Connectivity Protocol
//! Meshtastic Transport v1 (shared with the Meshtastic ingress via
//! [`crate::ingress::meshtastic::frame`]), and sends every fragment as
//! reliable (`want_ack = true`) unicast to a configured Meshtastic gateway
//! node over USB serial.
//!
//! This command reports **local submission**, not remote delivery: success
//! means every fragment was handed to the local radio, not that the gateway
//! has received or acknowledged it. Meshtastic firmware owns retransmission;
//! this command never retries a fragment itself.
//!
//! The actual serial I/O ([`RadioTransport`]) is kept behind the small
//! [`FragmentTransport`] trait so the fragment-submission loop
//! ([`submit_fragments`]) can be unit-tested with a fake, in-memory
//! transport — no real Meshtastic radio is required to run `cargo test`.

use super::{CliError, CliResult};
use crate::ingress::meshtastic::frame;
use crate::protocol;
use async_trait::async_trait;
use clap::{Args, Subcommand};
use meshtastic::api::{ConnectedStreamApi, StreamApi};
use meshtastic::packet::{PacketDestination, PacketReceiver, PacketRouter};
use meshtastic::protobufs::{from_radio, PortNum};
use meshtastic::types::{EncodedMeshPacketData, MeshChannel, NodeId};
use meshtastic::utils;
use std::convert::Infallible;
use std::time::Duration;

/// `edge sensor` subcommands.
#[derive(Debug, Subcommand)]
pub(crate) enum SensorCommand {
    /// Send an existing `SignedEnvelope` over Meshtastic for testing.
    Meshtastic(MeshtasticArgs),
}

/// `edge sensor meshtastic` arguments.
#[derive(Debug, Args)]
#[command(after_help = "\
EXAMPLES:
    # Full smoke-test pipeline: build, sign, and send telemetry over a local
    # Meshtastic radio to a specific gateway node.
    edge message -b urban temp=21.5 humidity=44 \\
      | edge envelope --sign \"$SENSOR_SURI\" \\
      | edge sensor meshtastic --device /dev/ttyACM0 --gateway '!deadbeef'

    # The gateway node id may also be given as 0x-prefixed hex or decimal.
    edge sensor meshtastic --device /dev/ttyACM0 --gateway 0xdeadbeef < env.bin")]
pub(crate) struct MeshtasticArgs {
    /// Local Meshtastic serial device (e.g. `/dev/ttyACM0`).
    #[arg(long, value_name = "PATH")]
    device: String,
    /// Destination Meshtastic gateway node: `!deadbeef`, `0xdeadbeef`, or a
    /// plain decimal node id.
    #[arg(long, value_name = "NODE_ID")]
    gateway: String,
    /// Connectivity Protocol `PortNum` fragments are sent on.
    #[arg(long, value_name = "PORT", default_value_t = 256)]
    port_num: u32,
}

/// How long to wait for the radio's configuration handshake (`MyInfo` +
/// matching `ConfigCompleteId`) before giving up.
const CONFIG_TIMEOUT: Duration = Duration::from_secs(15);

/// Dispatch a `sensor` subcommand.
pub(crate) fn run(command: SensorCommand) -> CliResult {
    match command {
        SensorCommand::Meshtastic(args) => run_meshtastic(args),
    }
}

/// Implements `edge sensor meshtastic`: parses arguments, validates stdin,
/// fragments the envelope, then hands fragments to the real serial
/// [`RadioTransport`].
fn run_meshtastic(args: MeshtasticArgs) -> CliResult {
    let gateway = parse_node_id(&args.gateway)
        .map_err(|e| CliError::usage(format!("invalid --gateway: {e}")))?;
    let port_num = PortNum::try_from(args.port_num as i32).map_err(|_| {
        CliError::usage(format!(
            "invalid --port-num {}: not a PortNum this Meshtastic SDK build can represent \
             (default 256 = PRIVATE_APP)",
            args.port_num
        ))
    })?;

    // For MVP, stdin is raw protobuf binary (no hex/base64 sniffing): the
    // exact bytes read here are the exact bytes fragmented and transmitted,
    // per Transport v1's `message_id = first_6_bytes(SHA256(raw_envelope))`
    // exact-byte-preservation requirement.
    let raw = super::format::read_stdin()?;
    // Structural validation only: reject obvious operator errors locally
    // before ever touching the radio. The envelope is never re-encoded.
    protocol::decode_envelope(&raw).map_err(super::format::protocol_err)?;

    let envelope_id = protocol::envelope_id(&raw).to_hex();
    eprintln!("envelope_id: {envelope_id}");

    let fragments =
        frame::encode_frames(&raw).map_err(|e| CliError::with_code(3, e.to_string()))?;
    eprintln!(
        "meshtastic_message_id: 0x{}",
        hex::encode(frame::message_id(&raw))
    );
    eprintln!("gateway: !{gateway:08x}");
    eprintln!("fragments: {}", fragments.len());

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::runtime(format!("failed to start async runtime: {e}")))?;

    runtime.block_on(send_over_serial(
        &args.device,
        gateway,
        port_num,
        &fragments,
    ))?;

    eprintln!("submitted {} fragments to !{gateway:08x}", fragments.len());
    Ok(())
}

/// Parse a Meshtastic node id given as `!deadbeef`, `0xdeadbeef`, or a plain
/// decimal number.
fn parse_node_id(input: &str) -> Result<u32, String> {
    let trimmed = input.trim();
    if let Some(hex_digits) = trimmed.strip_prefix('!') {
        return u32::from_str_radix(hex_digits, 16)
            .map_err(|e| format!("invalid node id {trimmed:?}: {e}"));
    }
    if let Some(hex_digits) = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex_digits, 16)
            .map_err(|e| format!("invalid node id {trimmed:?}: {e}"));
    }
    trimmed
        .parse::<u32>()
        .map_err(|e| format!("invalid node id {trimmed:?}: {e}"))
}

// ---------------------------------------------------------------------------
// Fragment transport abstraction
// ---------------------------------------------------------------------------

/// A one-fragment reliable-unicast send. Implemented once for the real
/// Meshtastic radio ([`RadioTransport`]) and once for tests
/// ([`tests::FakeTransport`]), so [`submit_fragments`] never needs real
/// hardware to be exercised.
#[async_trait]
trait FragmentTransport {
    /// Send one fragment to `destination` on `port_num`, with `want_ack`.
    async fn send_fragment(
        &mut self,
        fragment: Vec<u8>,
        port_num: PortNum,
        destination: PacketDestination,
        want_ack: bool,
    ) -> Result<(), String>;
}

/// Submit every fragment in order, stopping at the first failure. Every
/// fragment is sent to the same `destination`/`port_num` with
/// `want_ack = true` (Transport v1 is unicast-only; Meshtastic firmware owns
/// retransmission, so this loop never retries).
async fn submit_fragments<T: FragmentTransport>(
    transport: &mut T,
    fragments: &[Vec<u8>],
    port_num: PortNum,
    destination: PacketDestination,
) -> Result<(), (usize, String)> {
    let total = fragments.len();
    for (index, fragment) in fragments.iter().enumerate() {
        transport
            .send_fragment(fragment.clone(), port_num, destination, true)
            .await
            .map_err(|e| (index, e))?;
        eprintln!("fragment {}/{total}: submitted", index + 1);
    }
    Ok(())
}

/// Real [`FragmentTransport`]: submits fragments to a connected, configured
/// Meshtastic radio over serial.
struct RadioTransport<'a> {
    stream_api: &'a mut ConnectedStreamApi<meshtastic::api::state::Configured>,
    router: &'a mut SensorPacketRouter,
    channel: MeshChannel,
}

#[async_trait]
impl FragmentTransport for RadioTransport<'_> {
    async fn send_fragment(
        &mut self,
        fragment: Vec<u8>,
        port_num: PortNum,
        destination: PacketDestination,
        want_ack: bool,
    ) -> Result<(), String> {
        let packet_data: EncodedMeshPacketData = fragment.into();
        self.stream_api
            .send_mesh_packet(
                self.router,
                packet_data,
                port_num,
                destination,
                self.channel,
                want_ack,
                false, // want_response: this is a one-way telemetry send.
                false, // echo_response: no local echo needed.
                None,
                None,
            )
            .await
            .map_err(|e| e.to_string())
    }
}

/// Open the serial device, complete the Meshtastic configuration handshake,
/// and submit every fragment as reliable unicast to `gateway`. Stops after
/// the first local failure (no continuation, no retry).
async fn send_over_serial(
    device: &str,
    gateway: u32,
    port_num: PortNum,
    fragments: &[Vec<u8>],
) -> CliResult {
    let stream = utils::stream::build_serial_stream(device.to_string(), None, None, None)
        .map_err(|e| CliError::runtime(format!("failed to open serial device {device}: {e}")))?;

    let stream_api = StreamApi::new();
    let (mut decoded_rx, stream_api) = stream_api.connect(stream).await;
    let config_id = utils::generate_rand_id();
    let mut stream_api = stream_api
        .configure(config_id)
        .await
        .map_err(|e| CliError::runtime(format!("failed to configure meshtastic session: {e}")))?;

    let node_id = match wait_for_ready(&mut decoded_rx, config_id).await {
        Ok(node_id) => node_id,
        Err(e) => {
            let _ = stream_api.disconnect().await;
            return Err(CliError::runtime(e));
        }
    };
    let mut router = SensorPacketRouter { node_id };
    let channel =
        MeshChannel::new(0).expect("channel 0 is always a valid Meshtastic mesh channel index");
    let destination = PacketDestination::Node(NodeId::new(gateway));

    let mut transport = RadioTransport {
        stream_api: &mut stream_api,
        router: &mut router,
        channel,
    };

    let result = submit_fragments(&mut transport, fragments, port_num, destination).await;
    let _ = stream_api.disconnect().await;

    result.map_err(|(index, e)| {
        eprintln!("fragment {}/{}: failed: {e}", index + 1, fragments.len());
        CliError::runtime(format!(
            "failed to submit fragment {}/{} to the local radio: {e}",
            index + 1,
            fragments.len()
        ))
    })
}

/// Wait for the radio's `MyInfo` (our own node id) and a matching
/// `ConfigCompleteId`, bounded by [`CONFIG_TIMEOUT`].
async fn wait_for_ready(decoded_rx: &mut PacketReceiver, config_id: u32) -> Result<NodeId, String> {
    let deadline = tokio::time::Instant::now() + CONFIG_TIMEOUT;
    let mut node_id = None;
    let mut complete = false;

    while node_id.is_none() || !complete {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, decoded_rx.recv()).await {
            Ok(Some(packet)) => match packet.payload_variant {
                Some(from_radio::PayloadVariant::MyInfo(info)) => {
                    node_id = Some(NodeId::new(info.my_node_num));
                }
                Some(from_radio::PayloadVariant::ConfigCompleteId(id)) if id == config_id => {
                    complete = true;
                }
                _ => {}
            },
            Ok(None) => break,
            Err(_) => break,
        }
    }

    match node_id {
        Some(node_id) if complete => Ok(node_id),
        _ => Err("meshtastic configuration handshake timed out".to_string()),
    }
}

/// Minimal [`PacketRouter`] implementation: `edge sensor meshtastic` only
/// ever sends (with `echo_response = false`), so the packet-handling methods
/// are never invoked; they exist purely to satisfy the trait.
struct SensorPacketRouter {
    node_id: NodeId,
}

impl PacketRouter<(), Infallible> for SensorPacketRouter {
    fn handle_packet_from_radio(
        &mut self,
        _packet: meshtastic::protobufs::FromRadio,
    ) -> Result<(), Infallible> {
        Ok(())
    }

    fn handle_mesh_packet(
        &mut self,
        _packet: meshtastic::protobufs::MeshPacket,
    ) -> Result<(), Infallible> {
        Ok(())
    }

    fn source_node_id(&self) -> NodeId {
        self.node_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn parses_bang_hex_node_id() {
        assert_eq!(parse_node_id("!deadbeef").unwrap(), 0xdeadbeef);
    }

    #[test]
    fn parses_0x_hex_node_id() {
        assert_eq!(parse_node_id("0xDEADBEEF").unwrap(), 0xdeadbeef);
    }

    #[test]
    fn parses_decimal_node_id() {
        assert_eq!(parse_node_id("3735928559").unwrap(), 0xdeadbeef);
    }

    #[test]
    fn rejects_invalid_node_id() {
        assert!(parse_node_id("!zz").is_err());
        assert!(parse_node_id("not-a-node").is_err());
    }

    #[test]
    fn default_port_num_is_256() {
        use clap::CommandFactory;
        let cmd = crate::cli::Cli::command();
        let sensor = cmd
            .find_subcommand("sensor")
            .expect("sensor subcommand exists");
        let meshtastic = sensor
            .find_subcommand("meshtastic")
            .expect("meshtastic subcommand exists");
        let port_num_arg = meshtastic
            .get_arguments()
            .find(|a: &&clap::Arg| a.get_id() == "port_num")
            .expect("--port-num argument exists");
        assert_eq!(
            port_num_arg.get_default_values(),
            &["256"],
            "default --port-num must be 256"
        );
    }

    /// One recorded [`FragmentTransport::send_fragment`] call.
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FakeCall {
        fragment: Vec<u8>,
        port_num: i32,
        destination_node: Option<u32>,
        want_ack: bool,
    }

    /// An in-memory [`FragmentTransport`] that records every call instead of
    /// touching real hardware.
    #[derive(Default, Clone)]
    struct FakeTransport {
        calls: Arc<Mutex<Vec<FakeCall>>>,
    }

    #[async_trait]
    impl FragmentTransport for FakeTransport {
        async fn send_fragment(
            &mut self,
            fragment: Vec<u8>,
            port_num: PortNum,
            destination: PacketDestination,
            want_ack: bool,
        ) -> Result<(), String> {
            let destination_node = match destination {
                PacketDestination::Node(id) => Some(id.id()),
                _ => None,
            };
            self.calls.lock().unwrap().push(FakeCall {
                fragment,
                port_num: port_num as i32,
                destination_node,
                want_ack,
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn n_fragments_produce_n_unicast_calls() {
        let raw = vec![0x42u8; 1000]; // large enough to require several fragments
        let fragments = frame::encode_frames(&raw).expect("encodes");
        assert!(fragments.len() > 1, "expected a multi-fragment payload");

        let mut transport = FakeTransport::default();
        let destination = PacketDestination::Node(NodeId::new(0xdeadbeef));
        submit_fragments(&mut transport, &fragments, PortNum::PrivateApp, destination)
            .await
            .expect("all fragments submitted");

        let calls = transport.calls.lock().unwrap();
        assert_eq!(calls.len(), fragments.len());
        for (call, fragment) in calls.iter().zip(fragments.iter()) {
            assert_eq!(&call.fragment, fragment);
            assert_eq!(call.port_num, PortNum::PrivateApp as i32);
            assert_eq!(call.destination_node, Some(0xdeadbeef));
            assert!(call.want_ack, "every fragment must set want_ack = true");
        }
    }

    #[tokio::test]
    async fn single_fragment_envelope_produces_one_call() {
        let raw = b"small envelope".to_vec();
        let fragments = frame::encode_frames(&raw).expect("encodes");
        assert_eq!(fragments.len(), 1);

        let mut transport = FakeTransport::default();
        let destination = PacketDestination::Node(NodeId::new(1));
        submit_fragments(&mut transport, &fragments, PortNum::PrivateApp, destination)
            .await
            .expect("submitted");

        assert_eq!(transport.calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn submission_stops_after_first_failure() {
        /// A transport whose second call always fails, to verify that
        /// `submit_fragments` does not continue sending later fragments
        /// after a local failure.
        #[derive(Default)]
        struct FailingTransport {
            calls: usize,
        }

        #[async_trait]
        impl FragmentTransport for FailingTransport {
            async fn send_fragment(
                &mut self,
                _fragment: Vec<u8>,
                _port_num: PortNum,
                _destination: PacketDestination,
                _want_ack: bool,
            ) -> Result<(), String> {
                self.calls += 1;
                if self.calls == 2 {
                    Err("radio rejected fragment".to_string())
                } else {
                    Ok(())
                }
            }
        }

        let fragments = vec![vec![1u8], vec![2u8], vec![3u8]];
        let mut transport = FailingTransport::default();
        let destination = PacketDestination::Node(NodeId::new(1));
        let err = submit_fragments(&mut transport, &fragments, PortNum::PrivateApp, destination)
            .await
            .unwrap_err();
        assert_eq!(err.0, 1, "must fail at the second fragment (index 1)");
        assert_eq!(transport.calls, 2, "must not attempt the third fragment");
    }
}
