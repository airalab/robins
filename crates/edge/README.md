# Robonomics Edge Gateway

The `edge` is a **compact, single-binary** Connectivity Protocol ingress daemon for
SBC/edge devices, plus a full CLI toolbox for keys, envelopes and telemetry.
It accepts signed protocol envelopes over HTTP, validates them through one
canonical pipeline, and republishes accepted messages to a native libp2p
GossipSub topic — no database, no broker, no heavyweight runtime.

```text
  sensor ──▶ sign ──▶ HTTP POST ──▶ verify → auth → dedup ──▶ GossipSub ──▶ swarm
                                     ops server: /health /ready /metrics
```

## Why you'll like it

- 🪶 **One binary.** `edge` is both the daemon and the toolbox (`key`,
  `envelope`, `message`, `config`) — no separate client tools to install.
- 🔒 **Signed by default.** Every envelope is Ed25519-signed; the gateway
  verifies before it ever touches the pipeline.
- 🔌 **Unix-friendly.** Every command reads `stdin`/writes `stdout`, so it
  composes with `curl`, `jq`, and shell pipelines.
- 📡 **Batteries-included ops.** `/health`, `/ready`, `/metrics` (Prometheus)
  and `/debug/peers` ship out of the box — nothing to bolt on.

## Quick start

Install the binary from the workspace (Nix dev shell recommended):

```sh
nix develop --command cargo build -p edge --release
alias edge=./target/release/edge
```

### 1. Generate a sensor identity

```console
$ edge key generate
Secret seed:  0x40237f32074c4bd62cc257e30a5a43e65efe566be2e66ac85f807f7098009284
Public key:   0xedefe5b07e2c03b97f72748ef532d39fa2c724175216ac5a3c6d40cb7766b631
SS58 Address: 4Ha5tJacMSV2xSK5VvQ2naWP63EAFq1bD7HgNEdWJ9LYhj3U
```

### 2. Compose telemetry with compact argv measurements and sign it

`edge message` turns compact measurement arguments straight into a protobuf
payload — no schemas to hand-write:

```console
$ edge message --board urban temp=21.5 humidity=44 gps=51.5,-0.12,35
[M] Message
    |-- [#] node_id:   0
    |-- [T] timestamp: 1789660215567
    |-- [S] public:  bme280 temperature=21.50°C
    |-- [S] public:  bme280 humidity=44.00%
    `-- [S] public:  gps lat=51.50000 lon=-0.12000 height_m=35.0
```

Sign it into a wire-ready `SignedEnvelope` with the identity from step 1:

```console
$ edge message --board urban temp=21.5 humidity=44 > msg.bin
$ edge envelope --sign 0x4023...seed... --input binary < msg.bin > env.bin
```

### 3. Start the gateway

```console
$ edge config generate --output gateway.toml
wrote default configuration to gateway.toml

$ edge config check --config gateway.toml
configuration is valid

$ edge gateway --config gateway.toml
```

The daemon binds the HTTP ingress, subscribes to GossipSub, and starts the
ops server. `Ctrl-C` triggers a graceful shutdown.

### 4. Feed it and watch it work

```console
$ curl -s -o /dev/null -w 'HTTP %{http_code}\n' \
    -X POST http://127.0.0.1:3000/v1/telemetry --data-binary @env.bin
HTTP 202

$ curl -s http://127.0.0.1:9090/metrics
# HELP edge_ingress_received_total Total envelopes received across all ingress transports
# TYPE edge_ingress_received_total counter
edge_ingress_received_total{transport="http"} 1

# HELP edge_connected_peers Currently connected libp2p peers
# TYPE edge_connected_peers gauge
edge_connected_peers 0
```

One accepted envelope, end to end: signed on a sensor, verified and
deduplicated by the gateway, and counted in Prometheus metrics — all with the
one binary.

### 5. Meshtastic hardware smoke test (optional)

With a Meshtastic radio attached to the gateway host (and Meshtastic ingress
enabled in `gateway.toml`) and a second radio attached to the sensor host:

```console
$ edge message --board urban temp=21.5 humidity=44 \
    | edge envelope --sign 0x4023...seed... \
    | edge sensor meshtastic --device /dev/ttyACM0 --gateway '!deadbeef'
envelope_id: 9a8b...
meshtastic_message_id: 0x1a2b3c4d5e6f
gateway: !deadbeef
fragments: 1
fragment 1/1: submitted
submitted 1 fragments to !deadbeef
```

`edge sensor meshtastic` never builds or signs telemetry itself — it is a
transport sender that fragments an existing `SignedEnvelope` per the
Connectivity Protocol Meshtastic Transport v1 and unicasts it, with
`want_ack = true`, to the configured gateway node (accepted as `!deadbeef`,
`0xdeadbeef`, or a plain decimal id). Success here means every fragment was
*submitted to the local radio* — Meshtastic firmware owns retransmission, so
this is not a remote delivery confirmation.

## The toolbox at a glance

| Command                 | Alias | Purpose                                                |
| ------------------------ | ----- | ------------------------------------------------------- |
| `edge gateway`           | `g`   | Run the long-running ingress daemon.                   |
| `edge key generate`      | `k`   | Mint a fresh Ed25519 sensor identity (subkey-style).    |
| `edge key inspect <uri>` | `k`   | Report the public identity for a seed or SS58 address.  |
| `edge message`           | `m`   | Encode (compact argv) or decode a telemetry payload.   |
| `edge envelope`          | `e`   | Sign, verify, encode or decode a `SignedEnvelope`.      |
| `edge sensor meshtastic` | `s`   | Send an existing `SignedEnvelope` over Meshtastic (dev/hardware smoke test). |
| `edge config generate`   | `c`   | Emit a validated default `gateway.toml`.                |
| `edge config check`      | `c`   | Validate a configuration file.                          |

Every encode/decode command auto-detects its direction and byte format
(hex/base64/binary), so `edge envelope 0xaabb...` and
`cat file.bin | edge envelope --output hex` both just work.

## Operations endpoints

The ops server (bound to `[metrics].listen`) exposes:

| Path           | Purpose                                            |
| -------------- | --------------------------------------------------- |
| `/health`      | liveness probe (`ok` once the process is up)        |
| `/ready`       | readiness probe (gated on `min_connected_peers`)     |
| `/metrics`     | Prometheus exposition (ingress, pipeline, peers)     |
| `/debug/peers` | currently connected libp2p peers (diagnostics)       |

### Exit codes

Every command shares one exit-code contract, so scripts can branch reliably:

| Code | Meaning                                          |
| ---- | -------------------------------------------------- |
| `0`  | success                                            |
| `1`  | runtime error (bind failure, subsystem startup)    |
| `2`  | CLI usage / configuration error                    |
| `3`  | protocol / input error (malformed envelope)        |

## Configuration

Generate a validated default and edit it:

```sh
edge config generate --output /etc/edge/gateway.toml
```

A fully commented example lives at [`examples/gateway.toml`](examples/gateway.toml).

### Active sections

| Section      | Key                   | Default                     | Description                                            |
| ------------ | --------------------- | ---------------------------- | ------------------------------------------------------ |
| `[http]`     | `enabled`             | `true`                       | Enable the HTTP ingress transport.                     |
|              | `listen`              | `0.0.0.0:3000`               | Ingress bind address (`POST /v1/telemetry`).           |
|              | `max_body_bytes`      | `65536`                      | Reject request bodies larger than this.                |
| `[auth]`     | `mode`                | `none`                       | `none` accepts all valid envelopes; `whitelist` gates. |
|              | `file`                | —                            | Whitelist file (required for `whitelist`).             |
| `[pubsub]`   | `enabled`             | `true`                       | Enable the GossipSub publisher.                        |
|              | `listen`              | `["/ip4/0.0.0.0/tcp/64442"]` | libp2p listen multiaddrs (TCP or `/ws`).               |
|              | `topic`               | `sensors.social/v1`          | Topic accepted envelopes are published to.             |
|              | `reserved_peers`      | `[]`                         | Peers to dial and keep connected (TCP/`/ws`/`/wss`).   |
|              | `min_connected_peers` | `0`                          | Peers required before `/ready` reports ready.          |
|              | `identity_file`       | —                            | Persist the node identity for a stable peer id.        |
| `[metrics]`  | `listen`              | `127.0.0.1:9090`             | Operations server bind address.                        |
| `[pipeline]` | `ingress_buffer`      | `1024`                       | Ingress → pipeline channel capacity.                   |
|              | `dedup_capacity`      | `8192`                       | Max retained dedup entries.                            |
|              | `dedup_ttl_secs`      | `300`                        | Dedup entry time-to-live (seconds).                    |
| `[meshtastic]` | `enabled`           | `false`                      | Enable the Meshtastic serial ingress transport.        |
|              | `transport`           | `"serial"`                   | Radio transport kind; only `"serial"` is supported.    |
|              | `device`              | —                            | Serial device path (e.g. `/dev/ttyACM0`); required if enabled. |
|              | `port_num`            | `256` (`PRIVATE_APP`)        | Connectivity Protocol Meshtastic application `PortNum`. |
|              | `max_reassembled_bytes` | `4096`                     | Max bytes retained per reassembled envelope (protocol ceiling: `3376`). |
|              | `max_fragments`       | `16`                         | Max fragments per envelope (protocol ceiling: `16`).   |
|              | `max_pending`         | `128`                        | Max incomplete reassembly slots retained globally.     |
|              | `max_pending_per_sender` | `8`                       | Max incomplete reassembly slots retained per mesh sender. |
|              | `reassembly_timeout_secs` | `60`                     | Idle timeout for an incomplete assembly (seconds).     |
|              | `reassembly_absolute_timeout_secs` | `300`           | Absolute lifetime for an incomplete assembly (seconds). |
|              | `reconnect_min_secs`  | `1`                          | Minimum serial reconnect backoff (seconds).            |
|              | `reconnect_max_secs`  | `30`                         | Maximum serial reconnect backoff (seconds).            |

Only decoded packets with `pki_encrypted = true`, addressed to `port_num`, are
accepted as Connectivity Protocol ingress — this mirrors the PKI-authentication
requirement in the Transport v1 spec (`src/protobufs/transport/meshtastic/v1.md`).
The adapter owns its own connect/reconnect loop, so a missing or disconnected
radio never blocks daemon startup or the HTTP ingress path.

### Deferred sections

`[ipfs]`, `[blockchain]` and `[storage]` are parsed for forward
compatibility but have no effect in this build.

## Testing

```sh
nix develop --command cargo nextest run -p edge
```

`tests/e2e.rs` exercises the full HTTP → GossipSub path against a standalone
verifier node and asserts that a persisted identity yields a stable peer id
across a restart.
