# Launch your own node and grow a tree

> **Example:** `05_local_node_tree` &nbsp;·&nbsp; **Needs:** a local development node &nbsp;·&nbsp; **API:** `Cps::connect`, `Signer`, `submit`, `submit_finalized`, `set_meta`, `set_payload`, `delete`

Time to stop being a visitor. You will start your own Robonomics chain on your machine, with
pre-funded accounts, instant block production and a throwaway database, and build a CPS tree on it.

## Launch the node

There is no standalone Robonomics node binary. The
[airalab/robonomics](https://github.com/airalab/robonomics) repository builds the runtime and ships
a helper that runs it with the generic `polkadot-omni-node` as a single-node `--dev` chain. Its
developer guide ([Running a Development Chain](https://github.com/airalab/robonomics/blob/master/DEVELOPER_GUIDE.md#running-a-development-chain))
has the one-liners. In a checkout of that repository:

```bash
# build the runtime, generate the dev chain spec and bake both into a Docker image
nix develop -c ./scripts/development-chain.sh docker

# the script leaves the image untagged: tag the newest one and run it
docker tag "$(docker images -q | head -n 1)" robonomics-dev
docker run --rm -p 9944:9944 robonomics-dev
```

You get:

- JSON-RPC and WebSocket on `ws://127.0.0.1:9944`;
- block production without any session key setup;
- pre-funded development accounts `//Alice`, `//Bob`, `//Charlie`, ...;
- an ephemeral database: stop the container and the chain is gone.

Arguments after the image name replace its default command, so repeat the defaults when adding
flags: `docker run --rm -p 9944:9944 robonomics-dev --chain=/dev.json --dev --rpc-external`.

Prefer no Docker? `nix develop -c ./scripts/development-chain.sh run` runs the same chain as a
local process, and `cargo install staging-chain-spec-builder polkadot-omni-node` replaces Nix.

Check the node answers, with the CLI from Part II:

```bash
cargo run -p libcps -- status        # RPC: ws://127.0.0.1:9944, block numbers growing
```

## Grow a tree

The examples read two optional environment variables:

| Variable | Default | Meaning |
| --- | --- | --- |
| `ROBONOMICS_WS_URL` | `ws://127.0.0.1:9944` | node endpoint |
| `ROBONOMICS_SURI` | `//Alice` | signer secret URI |

```bash
cargo run -p libcps --example 05_local_node_tree --no-default-features
```

```rust,ignore
{{#include ../../../crates/libcps/examples/05_local_node_tree.rs:code}}
```

Walk through it:

1. **A signer** is created from a secret URI, `Signer::<Sr25519>::from_suri(..)`. `Cps` never stores
   it: you pass it to every submission, so one connection can serve many accounts.
2. **A root node** is created with metadata, a payload and the signer as its identity. Creating a
   root also creates a **scope** owned by the signer.
3. **`submit_finalized`** sends the transaction and waits until its block is finalized, returning a
   `TxReceipt` with the block hash and the result (the new `NodeId`).
4. **`submit`** returns as soon as the transaction is in the pool. The returned `PendingTransaction`
   can be awaited later with `wait_finalized()` (or `wait_in_block()` for the first inclusion), so
   other work can overlap with the wait.
5. **Reads** go through `cps.finalized()`, which pins them all to one block (see
   [Explore the network](02-explore-network.md)).
6. **Delete** children before their parent.

**CLI twin:** [`cps create`, `cps payload set`, `cps delete`](../cli/managing-nodes.md).

Next: [give the tree an identity](06-identity.md)
