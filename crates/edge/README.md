# Robonomics Edge Gateway

The `edge` is a compact, single-binary Connectivity Protocol ingress daemon for
SBC/edge devices. It accepts signed protocol envelopes over an HTTP transport,
validates them through a single canonical pipeline (signature → authorization →
deduplication), and republishes accepted messages to a native libp2p GossipSub
topic.

```text
  HTTP ingress ─▶ bounded channel ─▶ pipeline (verify → auth → dedup) ─▶ gossip
  ops server (/health, /ready, /metrics, /debug/peers)                  publisher
```

Durable storage, IPFS publishing, Robonomics anchoring and Meshtastic ingress
are deferred subsystems: their configuration is accepted but inactive in the
MVP build.

## Building

The workspace uses a Nix dev shell:

```sh
nix develop --command cargo build -p edge
```

## Running the gateway

```sh
# Validate configuration first (optional).
edge config check --config /etc/edge/gateway.toml

# Start the long-running daemon. Runs until SIGINT (Ctrl-C).
edge gw --config /etc/edge/gateway.toml
```

`--config` defaults to `/etc/edge/gateway.toml`. On startup the gateway binds
its listeners, subscribes to the GossipSub topic, and begins accepting
envelopes at `POST /v1/telemetry`. A `Ctrl-C` triggers a graceful shutdown that
drains and stops every subsystem.

### Exit codes

| Code | Meaning                                            |
| ---- | -------------------------------------------------- |
| `0`  | success                                            |
| `1`  | runtime error (bind failure, subsystem startup)    |
| `2`  | CLI usage / configuration error                    |
| `3`  | protocol / input error (malformed envelope)        |

## Operations endpoints

The operations server (bound to `[metrics].listen`) exposes:

| Path           | Purpose                                             |
| -------------- | --------------------------------------------------- |
| `/health`      | liveness probe (`ok` once the process is up)        |
| `/ready`       | readiness probe (gated on `min_connected_peers`)    |
| `/metrics`     | Prometheus exposition (ingress, pipeline, peers)    |
| `/debug/peers` | currently connected libp2p peers (diagnostics)      |

## Configuration

Generate a validated default and edit it:

```sh
edge config generate --output /etc/edge/gateway.toml
```

A fully commented example lives at [`examples/gateway.toml`](examples/gateway.toml).

### Active sections

| Section      | Key                   | Default                    | Description                                            |
| ------------ | --------------------- | -------------------------- | ------------------------------------------------------ |
| `[http]`     | `enabled`             | `true`                     | Enable the HTTP ingress transport.                     |
|              | `listen`              | `0.0.0.0:3000`             | Ingress bind address (`POST /v1/telemetry`).           |
|              | `max_body_bytes`      | `65536`                    | Reject request bodies larger than this.                |
| `[auth]`     | `mode`                | `none`                     | `none` accepts all valid envelopes; `whitelist` gates. |
|              | `file`                | —                          | Whitelist file (required for `whitelist`).             |
| `[pubsub]`   | `enabled`             | `true`                     | Enable the GossipSub publisher.                        |
|              | `listen`              | `["/ip4/0.0.0.0/tcp/64442"]` | libp2p listen multiaddrs (TCP or `/ws`).            |
|              | `topic`               | `sensors.social/v1`        | Topic accepted envelopes are published to.             |
|              | `reserved_peers`      | `[]`                       | Peers to dial and keep connected (TCP/`/ws`/`/wss`).   |
|              | `min_connected_peers` | `0`                        | Peers required before `/ready` reports ready.          |
|              | `identity_file`       | —                          | Persist the node identity for a stable peer id.        |
| `[metrics]`  | `listen`              | `127.0.0.1:9090`           | Operations server bind address.                        |
| `[pipeline]` | `ingress_buffer`      | `1024`                     | Ingress → pipeline channel capacity.                   |
|              | `dedup_capacity`      | `8192`                     | Max retained dedup entries.                            |
|              | `dedup_ttl_secs`      | `300`                      | Dedup entry time-to-live (seconds).                    |

### Deferred sections

`[meshtastic]`, `[ipfs]`, `[blockchain]` and `[storage]` are parsed for forward
compatibility but have no effect in this build.

## Testing

```sh
nix develop --command cargo nextest run -p edge
```

`tests/e2e.rs` exercises the full HTTP → GossipSub path against a standalone
verifier node and asserts that a persisted identity yields a stable peer id
across a restart.
