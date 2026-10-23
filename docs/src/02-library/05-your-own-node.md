# Launch your own node and grow a tree

> **Example:** `05_local_node_tree` &nbsp;·&nbsp; **Needs:** a local development node &nbsp;·&nbsp; **API:** `Cps::connect`, `Signer`, `submit`, `submit_finalized`, `set_meta`, `set_payload`, `delete`

Time to stop being a visitor. You will start your own Robonomics chain on your machine, with
pre-funded accounts, instant block production and a throwaway database, and build a CPS tree on it.

## Launch the node

Start a development chain exactly as described in the CLI part,
[Run a local development node](../01-cli/04-local-node.md): a single-node `--dev` chain on
`ws://127.0.0.1:9944` with pre-funded accounts `//Alice`, `//Bob`, ... and an ephemeral database.

Check the node answers:

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

**CLI twin:** [`cps create`, `cps payload set`, `cps delete`](../01-cli/05-managing-nodes.md).

Next: [give the tree an identity](06-identity.md)
