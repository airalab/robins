# Watch the network live

> **Example:** `03_watch` &nbsp;·&nbsp; **Needs:** internet &nbsp;·&nbsp; **API:** `Node::watch_payload`, `Node::watch_meta`

Reading is a snapshot; watching is a movie. Pick a node a sensor keeps updating, for example one of
the sensor.social nodes you found in the previous chapter, and follow it:

```bash
cargo run -p libcps --example 03_watch --no-default-features --features light-client -- <NODE_ID>
```

The example stays read-only, so it still needs no keys and no funds. Wait a few blocks and the
readings start to arrive, one line per finalized block that changed the node. `CPS_WATCH_COUNT` sets
how many updates to wait for (default 5).

```rust,ignore
{{#include ../../../crates/libcps/examples/03_watch.rs:code}}
```

- A watcher yields an update **per finalized block** that sets the field, with the block number and
  hash, so you always know *when* a reading was finalized. A `None` value means the field was
  cleared.
- `watch_meta()` works the same for metadata.
- `next()` returns `Some(Err(_))` for a problem with a single block and `None` when the
  subscription ends; decide per application whether to retry.

You have now seen the whole read side of `libcps`. From here on we write, which needs somewhere to
write to.

**CLI twin:** [`cps payload watch`](../cli/exploring.md#following-changes).

Next: [prepare transactions offline](04-offline-transactions.md)
