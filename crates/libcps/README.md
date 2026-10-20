# 🌳 libcps - Robonomics CPS Library & CLI

Rust library and command-line tool for the Robonomics **Cyber-Physical Systems (CPS)** pallet: a
tree of nodes with metadata, payload, identities and access scopes stored on the Robonomics chain.

| Package | For | Start here |
| --- | --- | --- |
| **`libcps`** (library) | builders writing Rust applications | [Part I of the book](../../docs/src/library/01-light-client.md), [numbered examples](examples/README.md) |
| **`cps`** (CLI binary) | devops and shell scripters | [Part II of the book](../../docs/src/cli/installation.md) |

The full guide is the documentation book in [`docs/`](../../docs/src/introduction.md): a journey
from exploring the live Robonomics Network with a light client, to launching your own node, building a
CPS tree, sharing access and encrypting data. Build it locally with `nix develop -c mdbook serve docs --open`.

## ✨ Features

- 🔧 **Type-safe CPS 1.1 API**: one `Cps` entry point, `Node`/`Address` handles, block-pinned reads,
  node identities, opaque `Transaction<T>` and an RPC-only client
- 💡 **Embedded light client** (`libcps::light`, feature `light-client`): no RPC provider needed
- 👀 **Watchers** for payload and metadata changes
- 🔑 **Scopes and capabilities** decided by the runtime, surfaced as typed errors
- 🔐 **Multi-algorithm AEAD encryption** (XChaCha20-Poly1305, AES-256-GCM, ChaCha20-Poly1305) with
  SR25519 and ED25519 keys
- 🖥️ **`cps` CLI** with profiles, JSON output and scripting-friendly exit codes

## 📦 Installation

```toml
[dependencies]
libcps = { version = "0.8.0", default-features = false }                    # RPC client only
# libcps = { version = "0.8.0", default-features = false, features = ["light-client"] }
```

| Feature | Default | Provides |
| --- | --- | --- |
| `cli` | yes | the `cps` binary (includes `light-client`) |
| `light-client` | via `cli` | embedded smoldot light node |

CLI:

```bash
cargo install libcps                    # from crates.io
nix run github:airalab/robins#libcps    # or with Nix, no install
```

## 🚀 Quick start

CLI (needs no node: `cps lc` serves an embedded light client on `127.0.0.1:9944`):

```bash
cps lc &                       # join the network
cps status                     # connection status
cps tree <NODE_ID> --depth 2   # explore a CPS tree
```

Library:

```rust
use libcps::{light::{LightNode, LightNodeConfig}, Cps, NodeId};

let light = LightNode::start(LightNodeConfig::default()).await?;
light.wait_ready().await?;
let cps = Cps::connect(light.rpc_url()).await?;

let at = cps.finalized().await?;                       // one consistent block
let node = at.node(NodeId::from(42));
println!("{:?} {:?}", node.meta().await?, node.payload().await?);
```

Writes are explicit: mutations build a `Transaction<T>` and nothing is sent until you call
`cps.submit_finalized(&tx, &signer)`. See [the examples](examples/README.md) and the book for the
rest: transactions, identities, scopes, encryption.

## 🛠️ Development

See [DEVELOPMENT.md](DEVELOPMENT.md).

```bash
cargo build --package libcps
cargo nextest run --package libcps
cargo check --package libcps --examples --all-features
cargo fmt && taplo fmt
```

Type-safe chain bindings come from the `robonomics-runtime-subxt-api` crate at build time: no
external code generation is needed.

## 🤝 Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## 📄 License

Apache-2.0

## 🔗 Links

- [Robonomics Network](https://robonomics.network)
- [Documentation](https://wiki.robonomics.network)
