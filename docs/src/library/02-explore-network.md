# Explore the network

> **Example:** `02_explore_network` &nbsp;·&nbsp; **Needs:** internet &nbsp;·&nbsp; **API:** `Cps::finalized`, `CpsAt`, `Node` reads, `resolve_scope`

You are connected. What is out there? The Robonomics Network is full of CPS trees built by real
devices. One of the most impressive is the **sensor.social** infrastructure: a community network of
environmental sensors whose nodes publish their measurements on chain. Because the chain is public
and your light client verifies it, you can read all of it anonymously.

```bash
cargo run -p libcps --example 02_explore_network --no-default-features --features light-client -- <ROOT_NODE_ID>
```

The root node comes from the first argument or from `CPS_ROOT_NODE`; without both, the example uses
the sensor.social root in its `SENSOR_SOCIAL_ROOT` constant. Set `CPS_MAX_DEPTH` to stop the walk
after a number of levels when the tree is large.

## One block, one consistent picture

The chain keeps moving while you read. Reading node after node from "the latest block" could mix the
states of different blocks. `cps.finalized()` therefore returns a **`CpsAt`**, a view pinned to one
finalized block. Every handle created from it (`at.node(id)`, `at.address(account)`,
`at.scope(id)`) reads that block only, so the printed tree is a snapshot.

```rust,ignore
{{#include ../../../crates/libcps/examples/02_explore_network.rs:code}}
```

What the example reads for every node:

| Call | Returns |
| --- | --- |
| `node.info()` | `None` if the node does not exist, otherwise its stored record |
| `node.resolve_scope()` | the scope the node belongs to, with its **owner** account |
| `node.meta()` / `node.payload()` | configuration / operational data, as bytes |
| `node.children()` | child node ids |

Values are plain bytes: sensor nodes usually store JSON text, while encrypted data shows up as hex
(see [Encrypt your data](08-encryption.md)).

> **Tip.** `cps.at(block_hash)` pins reads to any block your light client can still answer for, so
> you can look at the network as it was some blocks ago.

**CLI twin:** [`cps show` and `cps tree`](../cli/exploring.md).

Next: [watch a node change](03-watch.md)
