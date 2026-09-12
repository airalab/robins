# mqtt-bridge

MQTT bridge between IoT devices and the Robonomics Cyber-Physical Systems (CPS) pallet.

`mqtt-bridge` connects MQTT brokers to Robonomics blockchain nodes, forwarding
data in both directions in real time. It builds on top of [`libcps`](../libcps)
for blockchain access, cryptography, and node operations, and can be used either
as a standalone CLI or as a library from your own Rust application.

## Features

- **Bidirectional bridging**
  - **Subscribe mode**: listen to MQTT topics and update blockchain node payloads
  - **Publish mode**: monitor blockchain events and publish node changes to MQTT topics
  - **Config file mode**: manage many subscribe/publish bridges from a single TOML file
- **End-to-end encryption**: encrypt MQTT payloads before storing them on-chain and
  decrypt on-chain payloads before publishing (XChaCha20, AES-GCM-256, ChaCha20)
- **Multiple crypto schemes**: Sr25519 and Ed25519 (Home Assistant compatible)
- **Event-driven**: publish mode reacts to `PayloadSet` events on finalized blocks
  (no polling) and only publishes when the payload actually changes
- **Resilient**: auto-reconnects to both the broker and the blockchain on failure
- **Developer-friendly CLI**: colored output with timestamps and block numbers

## Installation

### Build from Source

```bash
# From the repository root
cargo build --release -p mqtt-bridge

# The binary will be available at
./target/release/mqtt-bridge
```

### Using Nix

```bash
nix develop
cargo build --release -p mqtt-bridge
```

## Configuration

All global options can be supplied via command-line flags or environment
variables.

| Flag                | Environment variable        | Default                  | Description                         |
| ------------------- | --------------------------- | ------------------------ | ----------------------------------- |
| `--ws-url`          | `ROBONOMICS_WS_URL`         | `ws://localhost:9944`    | Blockchain WebSocket URL            |
| `--suri`            | `ROBONOMICS_SURI`           | —                        | Account secret URI (e.g. `//Alice`) |
| `--log-level, -l`   | `RUST_LOG`                  | `warn`                   | Logging level                       |
| `--mqtt-broker`     | `ROBONOMICS_MQTT_BROKER`    | `mqtt://localhost:1883`  | MQTT broker URL                     |
| `--mqtt-username`   | `ROBONOMICS_MQTT_USERNAME`  | —                        | MQTT username                       |
| `--mqtt-password`   | `ROBONOMICS_MQTT_PASSWORD`  | —                        | MQTT password                       |
| `--mqtt-client-id`  | `ROBONOMICS_MQTT_CLIENT_ID` | generated                | MQTT client ID                      |

## Quick Start

### Subscribe: MQTT → Blockchain

Listen to an MQTT topic and update a blockchain node's payload with each message.

```bash
# Subscribe to sensor data, updating node 5
mqtt-bridge subscribe 'sensors/temp01' 5

# Subscribe with encryption (Sr25519)
mqtt-bridge subscribe 'sensors/temp01' 5 \
    --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY

# Subscribe with Ed25519 encryption (Home Assistant compatible)
mqtt-bridge subscribe 'homeassistant/sensor/temp' 5 \
    --receiver-public 5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY \
    --scheme ed25519 \
    --cipher aesgcm256
```

Arguments and options:

```
mqtt-bridge subscribe <TOPIC> <NODE_ID> [OPTIONS]

Arguments:
  <TOPIC>              MQTT topic to subscribe to
  <NODE_ID>            Node ID to update

Options:
  -r, --receiver-public <KEY>  Receiver public key or SS58 address (enables encryption)
      --cipher <CIPHER>        Encryption algorithm [default: xchacha20]
                               (xchacha20, aesgcm256, chacha20)
      --scheme <SCHEME>        Cryptographic scheme [default: sr25519] (sr25519, ed25519)
```

> Encryption requires `--suri` (or `ROBONOMICS_SURI`) to be set, since the
> sender's secret key is used to derive the shared encryption key.

### Publish: Blockchain → MQTT

Monitor a blockchain node for payload changes and publish them to an MQTT topic.

```bash
# Publish node 10 changes to a topic
mqtt-bridge publish 'actuators/valve01' 10

# Decrypt encrypted on-chain payloads before publishing
mqtt-bridge publish 'decrypted/sensor/data' 13 --decrypt
```

Arguments and options:

```
mqtt-bridge publish <TOPIC> <NODE_ID> [OPTIONS]

Arguments:
  <TOPIC>              MQTT topic to publish to
  <NODE_ID>            Node ID to monitor

Options:
  -d, --decrypt        Decrypt encrypted payloads before publishing
                       (algorithm and scheme are auto-detected)
```

### Config File Mode

Run many subscribe and publish bridges concurrently from a single TOML file.

```bash
mqtt-bridge start -c config.toml
```

Example configuration (see [`examples/mqtt_config.toml`](examples/mqtt_config.toml)
for the full version):

```toml
# MQTT Broker
broker = "mqtt://localhost:1883"
# username = "myuser"
# password = "mypass"
# client_id = "cps-bridge"

# Blockchain
[blockchain]
ws_url = "ws://localhost:9944"
suri = "//Alice"

# Listen to MQTT topics and update blockchain nodes
[[subscribe]]
topic = "sensors/temperature"
node_id = 5

# Subscribe with encryption
[[subscribe]]
topic = "sensors/pressure"
node_id = 7
receiver_public = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
cipher = "xchacha20"
scheme = "sr25519"

# Monitor blockchain nodes and publish changes to MQTT
[[publish]]
topic = "actuators/valve01"
node_id = 10

# Publish with auto-detected decryption
[[publish]]
topic = "decrypted/sensor/data"
node_id = 13
decrypt = true
```

The `[blockchain]` section is required when using `start`.

## Library Usage

`mqtt-bridge` can also be used as a library. Disable the default `cli` feature if
you only need the bridge API:

```toml
[dependencies]
mqtt-bridge = { version = "0.4", default-features = false }
```

### Subscribe bridge

```rust
use libcps::blockchain::Config as BlockchainConfig;
use mqtt_bridge as mqtt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let blockchain_config = BlockchainConfig {
        ws_url: "ws://localhost:9944".to_string(),
        suri: Some("//Alice".to_string()),
    };

    let mqtt_config = mqtt::Config {
        broker: "mqtt://localhost:1883".to_string(),
        username: None,
        password: None,
        client_id: Some("my-client".to_string()),
        blockchain: None,
        subscribe: Vec::new(),
        publish: Vec::new(),
    };

    // Optional custom handler invoked for every received message
    let handler = Box::new(|topic: &str, payload: &[u8]| {
        println!("Received on {}: {:?}", topic, payload);
    });

    mqtt_config
        .subscribe(
            &blockchain_config,
            None,          // cipher (None = no encryption)
            "sensors/temp",
            1,             // node_id
            None,          // receiver public key
            None,          // encryption algorithm
            Some(handler),
        )
        .await
}
```

### Loading a config file

```rust
use mqtt_bridge::Config;

let config = Config::from_file("examples/mqtt_config.toml")?;
config.start().await?; // Spawns all configured bridges
```

## Examples

Runnable examples live in [`examples/`](examples):

- [`mqtt_bridge.rs`](examples/mqtt_bridge.rs) — using the bridge from library code
- [`mqtt_config_load.rs`](examples/mqtt_config_load.rs) — loading and validating a config file
- [`mqtt_config.toml`](examples/mqtt_config.toml) — full configuration reference
- [`mqtt_encrypted.sh`](examples/mqtt_encrypted.sh) — encrypted subscribe bridge

```bash
cargo run --example mqtt_bridge
cargo run --example mqtt_config_load
```

## License

Apache-2.0
