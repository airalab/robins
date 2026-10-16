# libcps Examples

## Rust examples

Run from the repository root (inside `nix develop`):

### `light_client_read` - read the production chain with the light client

Starts the embedded light client (Polkadot relay chain + Robonomics parachain, specs
come from `robonomics-chain-spec`) and prints the whole CPS tree below a root node.
No RPC endpoint and no keys are needed; the first run waits until the light client
has synced a finalized block.

```bash
cargo run -p libcps --example light_client_read --no-default-features -- <ROOT_NODE_ID>
# or
CPS_ROOT_NODE=<ROOT_NODE_ID> cargo run -p libcps --example light_client_read --no-default-features
```

### `rpc_client_write` - send transactions to a local testnet over RPC

Creates a root node and a child, updates payload and metadata, reads them back and
deletes the nodes again.

1. Start a development node (the `robonomics` binary is built from the
   [airalab/robonomics](https://github.com/airalab/robonomics) repository and is not part of
   this workspace); it pre-funds `//Alice`:
   ```bash
   robonomics --dev --tmp
   ```
2. Run the example:
   ```bash
   cargo run -p libcps --example rpc_client_write --no-default-features
   ```

Environment variables (both optional):

- `ROBONOMICS_WS_URL` - node endpoint, default `ws://127.0.0.1:9944`
- `ROBONOMICS_SURI` - signer secret URI, default `//Alice`

## CLI examples (shell scripts)

Set `ROBONOMICS_WS_URL` (omit it to use the embedded light client) and `ROBONOMICS_SURI`
first, e.g. against the development node above:

```bash
export ROBONOMICS_WS_URL=ws://localhost:9944
export ROBONOMICS_SURI="//Alice"
```

- `encrypt_xchacha20.sh` - XChaCha20-Poly1305 encryption (default, recommended)
- `encrypt_ed25519.sh` - ED25519 scheme for IoT compatibility

MQTT bridge examples live in [`crates/mqtt-bridge/examples`](../../mqtt-bridge/examples).
