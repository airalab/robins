# Introduction

Robonomics is an open network for robots, sensors and smart cities. Its Cyber-Physical
Systems (CPS) pallet stores a **tree of nodes** on chain: every node has *metadata*
(configuration) and a *payload* (operational data), belongs to an *access scope*, and may be
bound to an *identity* account. Devices write their state into the tree, applications read
and watch it, and the chain decides who may change what.

This book is a journey with two parts that share one story:

1. **Explore the network.** Start a light client, connect to the live Robonomics Network
   without trusting anyone's RPC endpoint, browse real CPS nodes such as the
   [sensor.social](https://sensor.social) infrastructure and watch their readings arrive.
2. **Launch your own.** Run a local Robonomics node in Docker, create your own CPS tree, give
   it an identity, write data, hand another account write access and encrypt what you store.

| | Part I: Library | Part II: CLI |
| --- | --- | --- |
| For | **builders**: Rust developers writing applications | **devops**: operators and shell scripters |
| Tool | the `libcps` crate | the `cps` binary |
| Learn by | running numbered, runnable [examples](https://github.com/airalab/robins/tree/master/crates/libcps/examples) | copy-pasting commands |

Both parts follow the same chapters in the same order, so you can read one and use the other as a
translation table: whatever the library example does in Rust, the CLI does in one command.

## Before you start

- A checkout of this repository and a [Nix](https://nixos.org/download) shell with flakes:

  ```bash
  git clone https://github.com/airalab/robins
  cd robins
  nix develop
  ```

  Without Nix you need a Rust toolchain, `protoc` and `libclang`.
- Internet access for the exploration chapters (the light client syncs over p2p).
- Docker for the local node chapters.

## The mental model in 60 seconds

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

Ready? Start with [Join the network](library/01-light-client.md).
