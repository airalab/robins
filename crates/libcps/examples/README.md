# libcps Quickstart Tutorial

This tutorial walks through the runnable examples in this directory, from the simplest and
most used `libcps` calls to the more advanced ones. Each step names one example, the
core functions it teaches, how to run it and what to look for.

| # | Example | Needs | You learn |
| --- | --- | --- | --- |
| 1 | [`light_client_read`](#1-read-the-chain-light_client_read) | internet | connect, read nodes, pinned reads |
| 2 | [`offline_transactions`](#2-build-transactions-offline_transactions) | nothing | `Transaction<T>`, limits, `CreateNode` |
| 3 | [`rpc_client_write`](#3-write-to-a-node-rpc_client_write) | dev node | create, update, delete, submit |
| 4 | [`crypto_roundtrip`](#4-encrypt-your-data-crypto_roundtrip) | nothing | `Signer`, `Cipher`, encrypted payloads |
| 5 | [`watch_payload`](#5-follow-changes-watch_payload) | dev node | watchers |
| 6 | [`scope_access`](#6-scopes-and-access-scope_access) | dev node | scopes, grants, authorization, errors |
| 7 | [`address_identity`](#7-identities-and-address-access-address_identity) | dev node | identities, `Address`, snapshots |

Then see [CLI examples](#cli-examples-shell-scripts) and the [API cheat sheet](#api-cheat-sheet).

## Setup

Run everything from the repository root, inside the Nix shell:

```bash
nix develop
```

Examples 2 and 4 run fully offline and example 1 needs only internet access (light client).
Examples 3, 5, 6 and 7 write to a chain, so start a local development chain first. There is no
standalone node binary: the [airalab/robonomics](https://github.com/airalab/robonomics) repository
builds the runtime and ships a helper that runs it with `polkadot-omni-node` as a single-node
`--dev` chain (block production, RPC on `ws://127.0.0.1:9944`, pre-funded `//Alice`, `//Bob`, ...,
ephemeral database). See
[Running a Development Chain](https://github.com/airalab/robonomics/blob/master/DEVELOPER_GUIDE.md#running-a-development-chain)
in its developer guide. In a checkout of that repository, in a separate terminal:

```bash
nix develop -c ./scripts/development-chain.sh run
```

The first run builds the runtime WASM, which takes a while. Without Nix, install the tools with
`cargo install staging-chain-spec-builder polkadot-omni-node` and run
`./scripts/development-chain.sh run`. Stop the node with `Ctrl+C`; its state is discarded, so every
start gives you a fresh chain.

The write examples read two optional environment variables:

- `ROBONOMICS_WS_URL` - node endpoint, default `ws://127.0.0.1:9944`
- `ROBONOMICS_SURI` - signer secret URI, default `//Alice`

All examples run with `--no-default-features`, which skips the `cps` CLI dependencies:

```bash
cargo run -p libcps --example <name> --no-default-features
```

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

1. **Reads are `async` and immediate; mutations only build a `Transaction<T>`.** Nothing
   reaches the network until you pass the transaction and a signer to `cps.submit(..)` or
   `cps.submit_finalized(..)`.
2. **Authorization is decided by the runtime**, never by `libcps`. You ask (`has_capability`) or
   try and handle the typed error (`Error::AccessDenied`).

---

## 1. Read the chain: `light_client_read`

**Teaches:** `LightNode`, `Cps::connect`, `Cps::finalized`, `NodeAt` reads, `Result`/`Error`.

The simplest thing you can do with `libcps`: read. No endpoint, no keys. An embedded `LightNode`
serves a local RPC endpoint for the RPC-only `Cps`; it syncs the Polkadot relay chain and the Robonomics parachain over p2p (the specs ship in
`robonomics-chain-spec`), so the first run waits until a finalized block is available.

```bash
cargo run -p libcps --example light_client_read --no-default-features -- <ROOT_NODE_ID>
# or
CPS_ROOT_NODE=<ROOT_NODE_ID> cargo run -p libcps --example light_client_read --no-default-features
```

The core of it:

```rust
let light = LightNode::start(LightNodeConfig::default()).await?;
let cps = libcps::Cps::connect(light.rpc_url()).await?;  // or any RPC URL
let at = cps.finalized().await?;                   // pin one finalized block

let node = at.node(NodeId::from(root));
let owner = node.resolve_scope().await?.owner;     // scope & owner, computed by the runtime
let meta = node.meta().await?;                     // Option<Vec<u8>>
let payload = node.payload().await?;               // Option<Vec<u8>>
for child in node.children().await? { /* recurse */ }
```

Things to notice:

- Reads return `Option`: `None` means *unset* (or a missing node for `info`/`snapshot`).
- Everything read through `at` comes from the same block, so the printed tree is a consistent
  snapshot even while the chain moves on. Without `at`, `cps.node(id).payload()` reads the latest
  finalized block once per call.
- `Cps` is RPC-only: `Cps::connect(url)` works the same with a full node, a remote service or
  the local `LightNode`.

## 2. Build transactions: `offline_transactions`

**Teaches:** `Transaction<T>`, `create_root`, the `CreateNode` builder, size limits, identity
assignment parameters.

Before writing to a chain, see what a transaction *is*. This example needs no connection:
a `Transaction<T>` is a plain prepared value that never touches the network.

```bash
cargo run -p libcps --example offline_transactions --no-default-features
```

```rust
let tx: Transaction<NodeId> = libcps::create_root(
    CreateNode::new().meta(br#"{"type":"building"}"#).payload(br#"{"status":"online"}"#),
)?;
```

Expected output (abridged):

```text
limits: meta 1024 bytes, payload 8192 bytes
rejected: payload is 8193 bytes, the maximum is 8192
```

Things to notice:

- `T` is the *result* you get after finalization: `NodeId` for node creation, `ScopeId` for scope
  creation and `()` otherwise.
- Oversized meta/payload fail **when the transaction is built** (`Error::MetaTooLarge`,
  `Error::PayloadTooLarge`), so nothing oversized is ever sent.
- `IdentityAssignment::new(account)` (no permit) is for the signer's own account;
  `IdentityAssignment::with_permit(account, permit)` is for any other account. Identities are
  covered in step 7.
- The `Node` handle builds transactions the same way (`cps.node(id).set_payload(..)`), but needs a
  `Cps`, so those appear from step 3 on.

## 3. Write to a node: `rpc_client_write`

**Teaches:** `Cps::connect`, `create_root`, `create_child`, `set_payload`/`set_meta`, `delete`,
`submit` vs `submit_finalized`, `TxReceipt`.

The classic write flow against a local dev node: create a tree, update it, read it back, delete it.

```bash
cargo run -p libcps --example rpc_client_write --no-default-features
```

```rust
let signer = Signer::<Sr25519>::from_suri("//Alice")?;
let cps = Cps::connect("ws://127.0.0.1:9944").await?;

// 1. build (no network) ...
let tx = libcps::create_root(CreateNode::new().meta(b"HQ"))?;
// 2. ... then submit with an explicit signer and wait for finality
let receipt = cps.submit_finalized(&tx, &signer).await?;
let root = receipt.result;                          // NodeId of the new node

let child = cps
    .submit_finalized(&cps.node(root).create_child(CreateNode::new())?, &signer)
    .await?
    .result;

// Fire and forget, finalize later
let pending = cps.submit(&cps.node(child).set_payload(b"24C")?, &signer).await?;
println!("extrinsic {:?}", pending.extrinsic_hash());
let receipt = pending.wait_finalized().await?;      // TxReceipt { extrinsic_hash, block_hash, result }
```

Things to notice:

- `submit_finalized` waits for finalization **and** successful execution; a rejected call
  surfaces as an `Err` (for example `Error::AccessDenied`).
- `submit` returns a `PendingTransaction<T>` as soon as the extrinsic is in the pool, so other
  work can run before `wait_finalized()`.
- The signer is an argument of `submit*`; `Cps` never holds one. Use `Signer::<Ed25519>` for
  ED25519 accounts.
- Children must be deleted before their parent; a node with children cannot be deleted.
- The example ends with reads through a pinned `CpsAt`, and with a read by account (`address`),
  which step 7 explains.

## 4. Encrypt your data: `crypto_roundtrip`

**Teaches:** `Signer<S>`, `PublicKey<S>`, `Cipher::encrypt`/`decrypt`, `EncryptionAlgorithm`,
`EncryptedMessage`.

Node meta and payload are public on chain. To keep data private, encrypt before storing. This
example runs offline.

```bash
cargo run -p libcps --example crypto_roundtrip --no-default-features
```

```rust
use libcps::crypto::{Cipher, EncryptionAlgorithm, Signer, Sr25519};

let alice = Signer::<Sr25519>::from_suri("//Alice")?;
let bob = Signer::<Sr25519>::from_suri("//Bob")?;

let message = alice.encrypt(b"secret", &bob.public_key(), EncryptionAlgorithm::XChaCha20Poly1305)?;
let plain = bob.decrypt(&message, Some(&alice.public_key()))?;   // Some(sender) pins the sender
```

To store the message on chain, SCALE-encode it and use it as a payload; decode it after reading:

```rust
use parity_scale_codec::{Decode, Encode};

let tx = cps.node(id).set_payload(message.encode())?;             // write
let stored = cps.node(id).payload().await?.unwrap();
let message = EncryptedMessage::decode(&mut stored.as_slice())?;  // read
```

Expected output (abridged):

```text
[sr25519] xchacha20poly1305: 91 bytes encrypted, decrypted ok
wrong sender rejected: Sender public key mismatch: message from unexpected sender
tampered message rejected: XChaCha20 decryption failed: aead::Error
```

Things to notice:

- A `Signer<S>` is tagged with its scheme (`Sr25519` or `Ed25519`) and so is its `PublicKey<S>`.
  Mixing schemes is a **compile error**, not a runtime failure.
- Keys are agreed with ECDH and derived with HKDF; the same keypair identity signs transactions
  but transactions are only ever signed with the asymmetric signing key.
- Three AEAD algorithms are supported (`XChaCha20Poly1305` default, `AesGcm256`,
  `ChaCha20Poly1305`). Tampering or a wrong sender is detected on decryption.
- An encrypted payload is larger than the plaintext (nonce + tag + header), which counts against
  the 8192 byte payload limit.

## 5. Follow changes: `watch_payload`

**Teaches:** `Node::watch_payload`, `Node::watch_meta`, running a reader and a writer together.

Instead of polling, subscribe. A watcher yields an update for every finalized block that changes
the field, with the block hash and number.

```bash
cargo run -p libcps --example watch_payload --no-default-features
```

```rust
let mut watcher = cps.node(node).watch_payload().await?;
while let Some(update) = watcher.next().await {
    let update = update?;
    println!("block {}: {:?}", update.block_number, update.payload);  // None = payload cleared
}
```

Things to notice:

- The watcher starts at the next block, so create it **before** the write you want to observe.
- `update.payload == None` means the payload was cleared.
- This is what the `mqtt-bridge` blockchain-to-MQTT direction is built on.

## 6. Scopes and access: `scope_access`

**Teaches:** `resolve_scope`, `create_scope` vs `replace_scope`, `grant`/`revoke`, `Grant`,
`has_capability`, typed authorization errors.

Ownership in CPS belongs to a *scope*, not to a node. A scope is rooted at a node and owned by one
account; nodes without their own scope resolve to the scope of the nearest ancestor that has one.
Other accounts get *grants* inside a scope.

```bash
cargo run -p libcps --example scope_access --no-default-features
```

```rust
let node = cps.node(room);

// Who owns what? The runtime computes this.
let scope = node.resolve_scope().await?;           // id, root, owner, path
let record = cps.scope(scope.id).info().await?;    // owner, access_count

// Nested scope. Refused if the node already roots one ...
let id = cps.submit_finalized(&node.create_scope(), &owner).await?.result;
// ... replacing is destructive, so it has its own explicit call
let id = cps.submit_finalized(&node.replace_scope(), &owner).await?.result;

// Grants: Node mode = this node only, Subtree mode = also descendants in the same scope
cps.submit_finalized(&node.grant(bob, Grant::write_subtree())?, &owner).await?;
let allowed = node.has_capability(&bob, Capability::Write).await?;
cps.submit_finalized(&node.revoke(bob, Capability::Write)?, &owner).await?;
```

Things to notice:

- `create_scope` fails with `Error::ScopeAlreadyExists` on an existing scope root. Use
  `replace_scope` when you really mean it: it gives the node a fresh `ScopeId` and drops the old
  access entries. The guard is checked when the transaction is submitted, so it is a best-effort
  safeguard, not an atomic runtime check.
- `Grant` pairs a `Capability` (`Write`, `CreateScope`) with a `GrantMode` (`Node`, `Subtree`);
  shortcuts: `Grant::write_node()`, `Grant::write_subtree()`, `Grant::create_scope_node()`,
  `Grant::create_scope_subtree()`.
- A subtree grant stops at a nested scope boundary.
- Capability does not mean everything: in the example Bob may write the desk but his attempt to
  delete it fails with `Error::AccessDenied`. Match on the error to react:

  ```rust
  match cps.submit_finalized(&tx, &bob).await {
      Err(Error::AccessDenied) => { /* not allowed */ }
      other => { other?; }
  }
  ```
- `has_capability` on a missing node fails with `Error::NodeNotFound`.

## 7. Identities and address access: `address_identity`

**Teaches:** CPS 1.1 node identities, `Address`/`AddressAt`, explicit `NodeId` ↔ `AccountId`
conversion, `snapshot`, `set_identity`/`clear_identity`, strict address projection errors.

The most advanced step. CPS has **one tree with two ways to address it**: the canonical `NodeId`
and the identity `AccountId` a node may be bound to (think: a device's own account). Each
interface returns values in its own address space; the conversion is explicit and fallible.

```bash
cargo run -p libcps --example address_identity --no-default-features
```

```rust
// Bind an identity at creation. The signer's own account needs no permit.
let tx = libcps::create_root(
    CreateNode::new().identity(IdentityAssignment::new(signer.account_id())),
)?;

// Two interfaces, two address spaces
let parent: Option<NodeId> = cps.node(id).parent().await?;
let parent: Option<AccountId> = cps.address(account).parent().await?;

// Explicit conversion (no From impls between the two)
let identity: Option<AccountId> = cps.node(id).identity().await?;
let node_id: Option<NodeId> = cps.address(account).node_id().await?;

// Change identity with explicit transactions
cps.submit_finalized(&cps.node(id).set_identity(assignment)?, &owner).await?;
cps.submit_finalized(&cps.node(id).clear_identity(), &owner).await?;
```

For a child node with *another* account's identity, that account must consent with a permit
(`IdentityAssignment::with_permit(account, permit)`). The signature is made by the identity
account over the permit payload that the runtime defines; `libcps` does not construct it.

Things to notice:

- **Strict projection.** `cps.address(a).children()` fails with `Error::MissingProjectedIdentity` if
  any child has no identity, instead of silently omitting it. An account bound to no node fails with
  `Error::IdentityNotFound`, while `node_id()` simply returns `None`.
- `Address` is read-only: mutations always go through `Node`.
- `at.node(id).snapshot()` returns a `NodeSnapshot { id, parent, scope, identity, meta, payload }`
  read at one block. `None` means the node does not exist; `Some` with `meta: None` means it exists
  but meta is unset.
- Runtime identity rejections are typed: `Error::Identity(IdentityError::AlreadyInUse | NotSet |
  PermitRequired | PermitExpired | PermitTooLong | InvalidPermit | RootIdentityMustBeCaller)`.
- Pinned reads work for addresses too: `cps.finalized().await?.address(account).payload()`.

---

## What's next

- Combine steps 3 + 4: encrypt a payload, store it, read it back and decrypt it
  (this is what the CLI does with `--receiver-public`).
- Run production-style bridging with the [`mqtt-bridge`](../../mqtt-bridge/examples) examples.
- Use the `cps` CLI for quick manual work (`cps show`, `cps create`, `cps set-payload`, `cps remove`).
- Read the API reference: `cargo doc -p libcps --no-deps --open`.
- Live integration tests that exercise these flows: `tests/scope_access.rs`,
  `tests/identity_address.rs` (see `DEVELOPMENT.md`).

## CLI examples (shell scripts)

Set `ROBONOMICS_WS_URL` (default `ws://127.0.0.1:9944`; run `cps lc` for a local light client) and `ROBONOMICS_SURI`
first, e.g. against the development node above:

```bash
export ROBONOMICS_WS_URL=ws://localhost:9944
export ROBONOMICS_SURI="//Alice"
```

- `shell/encrypt_xchacha20.sh` - XChaCha20-Poly1305 encryption (default, recommended)
- `shell/encrypt_ed25519.sh` - ED25519 scheme for IoT compatibility

MQTT bridge examples live in [`crates/mqtt-bridge/examples`](../../mqtt-bridge/examples).

## API cheat sheet

Ordered like the tutorial, from the most used to the most advanced.

| Task | Call |
| --- | --- |
| Connect (RPC) | `Cps::connect(url)` (local light node: `LightNode::start(..)`, then `light.rpc_url()`) |
| Read a node | `cps.node(id).info() / meta() / payload() / children() / parent()` |
| Consistent multi-read | `let at = cps.finalized().await?` (or `cps.at(hash)`), then `at.node(id)...` |
| Create / update / delete | `libcps::create_root(..)`, `node.create_child(..)`, `set_meta`, `set_payload`, `clear_*`, `delete` |
| Send | `cps.submit_finalized(&tx, &signer)`, `cps.submit(&tx, &signer)` + `wait_finalized()` |
| Encrypt | `signer.encrypt(data, &receiver_public, algorithm)`, `receiver.decrypt(&msg, Some(&sender_public))` |
| Follow changes | `node.watch_payload()`, `node.watch_meta()` |
| Scope | `node.resolve_scope()`, `cps.scope(id).info()`, `create_scope()`, `replace_scope()` |
| Access | `node.grant(account, Grant::..)`, `revoke(account, cap)`, `has_capability(&account, cap)` |
| Identity | `CreateNode::identity(..)`, `set_identity(..)`, `clear_identity()`, `node.identity()` |
| By account | `cps.address(account).node_id() / parent() / children() / payload() / scope_id()` |
| Missing vs unset | `node.snapshot()` |
