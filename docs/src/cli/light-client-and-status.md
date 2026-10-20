# Light client and status

> **Library twin:** [Join the network](../library/01-light-client.md)

## `cps light-client` (`cps lc`)

No node of your own? Run the embedded light client in a terminal and leave it running:

```bash
cps lc
```

It syncs the Polkadot relay chain and the Robonomics parachain from scratch and serves a local
WebSocket JSON-RPC endpoint on `127.0.0.1:9944`, the default endpoint of every other `cps` command.
Stop it with `Ctrl+C`.

| Option | Meaning |
| --- | --- |
| `--listen <ADDR>` | endpoint address, default `127.0.0.1:9944` |
| `--allow-external` | permit a non-loopback address; the endpoint has **no authentication**, enable it only on trusted networks |

```bash
cps lc --listen 127.0.0.1:9955            # another port
cps --ws-url ws://127.0.0.1:9955 show 1    # use it from another terminal
```

Any Substrate RPC client can use this endpoint, not only `cps`.

## `cps status`

`cps status` answers "where am I connected, and is it alive?":

```bash
cps status
```

It prints the **effective configuration** (profile, config file, RPC endpoint) and live chain
information: chain name, genesis hash, runtime version, best and finalized block, and whether the node
is syncing (`unknown` when the endpoint does not say). It works the same against a full node, a remote
RPC service and `cps lc`.

If the endpoint is unreachable the configuration is still printed and the exit status is `2`, so the
command doubles as a health check:

```bash
cps status || echo "chain is not reachable"
cps --output json status          # machine-readable document
```

While a light client is still syncing, `cps status` shows how far it got; wait until the finalized block
is close to the best block before reading data.
