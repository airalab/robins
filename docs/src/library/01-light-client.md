# Join the network: the light client

> **Example:** `01_light_client` &nbsp;·&nbsp; **Needs:** internet &nbsp;·&nbsp; **API:** `light::LightNode`, `LightStatus`, `Cps::connect`

Our journey starts where every Robonomics traveler starts: by connecting. You do not need an
account, funds, or somebody else's RPC server. `libcps` embeds a
[smoldot](https://github.com/smol-dot/smoldot) **light client**. It syncs the Polkadot relay chain
and the Robonomics parachain over p2p (the chain specs ship in the `robonomics-chain-spec` crate)
and serves a local JSON-RPC endpoint. The RPC-only `Cps` client then connects to that endpoint, so
the data you read is verified by the light client instead of being trusted from a remote host.

```bash
cargo run -p libcps --example 01_light_client --no-default-features --features light-client
```

The light node is stateless: every run syncs from scratch and takes a little while until the first
finalized block is known.

## What to look for

`LightNode::status()` is a [watch channel](https://docs.rs/tokio/latest/tokio/sync/watch/index.html)
holding the connection status. A healthy start moves through:

| Status | Meaning |
| --- | --- |
| `starting` | chains are initializing, the endpoint is not listening yet |
| `rpc-ready` | the endpoint answers requests, sync progress is not known yet |
| `syncing (finalized #N, target #M, P peers)` | following the parachain; peers and progress are visible |
| `ready` | synchronized enough to answer finalized-state queries |

`stopped` and `failed` are terminal. A node may fall back from `ready` to `syncing` if it loses
pace, so long-running applications should keep watching the status.

## The code

```rust,ignore
{{#include ../../../crates/libcps/examples/01_light_client.rs:code}}
```

- `LightNodeConfig::default()` listens on a free loopback port; use `.listen("127.0.0.1:9944")`
  for a fixed one. Non-loopback addresses must be allowed explicitly with
  `.allow_non_loopback(true)` because the endpoint has no authentication.
- If you only need to wait until the node is usable, `light.wait_ready().await?` does the loop above
  for you; the next chapters use it.
- `Cps::connect` takes **any** WebSocket URL, so the rest of the book works the same against a
  public RPC endpoint or your own node.

**CLI twin:** [`cps light-client` and `cps status`](../cli/light-client-and-status.md).

Next: [what is on the network?](02-explore-network.md)
