# Introduction

<div class="robins-hero">
  <p class="robins-hero__eyebrow">Robonomics / Robins</p>
  <p class="robins-hero__tagline">Engineering the Open Machine Economy.</p>
  <p class="robins-hero__lead">Technical building blocks for Robonomics infrastructure: blockchain
  access, CPS tooling, edge connectivity and MQTT integration.</p>
</div>

Robonomics is an open network for robots, sensors and smart cities. Its Cyber-Physical
Systems (CPS) pallet stores a **tree of nodes** on chain: every node has *metadata*
(configuration) and a *payload* (operational data), belongs to an *access scope*, and may be
bound to an *identity* account. Devices write their state into the tree, applications read
and watch it, and the chain decides who may change what.

This book gets you from zero to a working setup with the `cps` command-line tool and the `libcps`
Rust library.

## Quick start

No checkout and no account needed:

1. **Install** the `cps` binary from the newest nightly release
   (other options, such as `cargo install` and Nix, are in [Installation](01-cli/01-installation.md)):

   ```bash
   # pick one of: linux-x86_64, linux-aarch64, macos-aarch64, macos-x86_64
   PLATFORM=linux-x86_64
   TAG=$(gh release list --repo airalab/robins --limit 1 --json tagName --jq '.[0].tagName')
   curl -fsSL https://github.com/airalab/robins/releases/download/$TAG/robins-$TAG-$PLATFORM.tar.gz | tar -xz
   sudo install -m 755 cps /usr/local/bin/
   ```

2. **Join the network** with the embedded light client. It trusts no one's RPC endpoint and keeps
   running, so leave it in its own terminal:

   ```bash
   cps lc
   ```

3. **Look around** from another terminal, once `cps status` shows the chain is synced:

   ```bash
   cps status
   cps tree <NODE_ID> --depth 2   # any node id, see "Exploring the tree"
   ```

Next, [Light client and status](01-cli/02-light-client-and-status.md) explains what you just ran.

## Choose your path

| | Part I: CLI | Part II: Library |
| --- | --- | --- |
| For | **devops**: operators and shell scripters | **builders**: Rust developers writing applications |
| Tool | the `cps` binary | the `libcps` crate |
| Learn by | copy-pasting commands | running numbered, runnable [examples](https://github.com/airalab/robins/tree/master/crates/libcps/examples) |
| Start with | [Installation](01-cli/01-installation.md) | [Join the network](02-library/01-light-client.md) |

Start with the CLI even if you are a builder: it needs nothing but a binary. The library part
retells the same story in Rust, so whatever a library example does, the CLI does in one command;
each chapter links to its twin.

The book follows one story:

1. **Explore the network.** Connect to the live Robonomics Network, browse real CPS nodes such as
   the [sensor.social](https://sensor.social) infrastructure and watch their readings arrive.
2. **Launch your own.** Run a local Robonomics node in Docker, create your own CPS tree, give it
   an identity, write data, hand another account write access and encrypt what you store.

## Before you go further

- Internet access for the exploration chapters (the light client syncs over p2p).
- Docker for the local node chapters.
- For the library part and building from source, a checkout of this repository and a
  [Nix](https://nixos.org/download) shell with flakes:

  ```bash
  git clone https://github.com/airalab/robins
  cd robins
  nix develop
  ```

  Without Nix you need a Rust toolchain, `protoc` and `libclang`.


## The mental model in 60 seconds (library)

The `libcps` API in one picture; the CLI commands map onto it one to one.

```text
Cps                    one connection, the only entry point (never stores a signer)
 ├─ node(NodeId)       Node     canonical address of a node in the CPS tree
 ├─ address(AccountId) Address  same tree, addressed by identity account (read-only)
 ├─ scope(ScopeId)     Scope    ownership/access boundary
 ├─ finalized()/at(h)  CpsAt    reads pinned to ONE block
 └─ submit(tx, signer)          the only call that sends anything
Transaction<T>         a prepared, unsigned mutation; T is its result (NodeId, ScopeId or ())
```

Two rules explain almost every example:

1. **Reads are `async` and immediate; mutations only build a `Transaction<T>`.** Nothing reaches
   the network until you pass the transaction and a signer to `cps.submit(..)` or
   `cps.submit_finalized(..)`.
2. **Authorization is decided by the runtime**, never by `libcps`. You ask (`has_capability`)
   or try and handle the typed error (`Error::AccessDenied`).

Ready? Start with [Installation](01-cli/01-installation.md), or jump to the
[library](02-library/01-light-client.md) if you write Rust.

<div class="chain-divider" role="presentation"></div>

<small>Robonomics identity artwork: Anna Wimmer-Savinova, © 2026 Anna Wimmer-Savinova. Used under
the Robonomics community-use terms, not under the Apache-2.0 license of the source code; see the
[brand notice](https://github.com/airalab/robins/blob/master/docs/src/assets/brand/BRAND-NOTICE.md).</small>
