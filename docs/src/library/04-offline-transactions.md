# Prepare transactions offline

> **Example:** `04_offline_transactions` &nbsp;·&nbsp; **Needs:** nothing &nbsp;·&nbsp; **API:** `create_root`, `CreateNode`, `Transaction<T>`, `IdentityAssignment`

Before touching a chain, learn how `libcps` models writes. A mutation is a **prepared value**:
`libcps::create_root(..)`, `node.set_payload(..)`, `node.grant(..)` and friends only *build* a
`Transaction<T>`. Building never touches the network and needs no signer, so it can be tested
offline and sent later, possibly by another component that holds the key.

`T` is the result you get once the transaction is finalized: the new `NodeId` for node creation, a
`ScopeId` for scope creation and `()` otherwise.

```bash
cargo run -p libcps --example 04_offline_transactions --no-default-features
```

```rust,ignore
{{#include ../../../crates/libcps/examples/04_offline_transactions.rs:code}}
```

Things to notice:

- **Limits are checked at build time.** `MAX_META_SIZE` and `MAX_PAYLOAD_SIZE` mirror the runtime,
  so an oversized value fails with `Error::PayloadTooLarge` before anything is sent.
- **Builders take anything byte-like**: `&str`, `Vec<u8>`, byte-string literals.
- **Identity assignment** is part of node creation. The signer's own account needs no permit; any
  other account must bring a signed permit (more in [Identities and addresses](06-identity.md)).

Next: [launch your own node](05-your-own-node.md)
