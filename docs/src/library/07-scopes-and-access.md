# Scopes and access

> **Example:** `07_scope_access` &nbsp;·&nbsp; **Needs:** a local development node &nbsp;·&nbsp; **API:** `resolve_scope`, `create_scope`, `Grant`, `Capability`, `has_capability`, `Error::AccessDenied`

Your tree is only yours so far. In CPS, ownership belongs to a **scope**, not to a node. Every node
resolves to the scope of its nearest ancestor that roots one, and the scope's owner decides who else
may act inside it. In this chapter `//Alice` owns a tree and lets `//Bob` write into it.

```bash
cargo run -p libcps --example 07_scope_access --no-default-features
```

```rust,ignore
{{#include ../../../crates/libcps/examples/07_scope_access.rs:code}}
```

## Concepts

- **Scope.** A root node creates one. `node.resolve_scope()` tells which scope a node belongs to,
  its root and its owner. `node.create_scope()` makes a node the root of a *nested* scope; it is
  refused with `Error::ScopeAlreadyExists` for a node that already roots one.
- **Capability.** What an account may do: `Capability::Write` (change data in the scope) and
  `Capability::CreateScope`.
- **Grant mode.** `Grant::write_node()` covers one node; `Grant::write_subtree()` also covers its
  descendants in the scope. Granting and revoking is done by the owner.
- **The runtime is the judge.** `libcps` never decides authorization itself. Ask with
  `has_capability(&account, capability)`, or just try: a denied transaction fails with the typed
  `Error::AccessDenied`.

## What the example proves

1. Bob cannot write at first.
2. After `write_node` on the room, Bob can write the room but not the desk below it; after
   `write_subtree` he can write both.
3. A transaction **signed by Bob** (`submit_finalized(&tx, &bob)`) changes the payload: data written
   from a different account lands on Alice's tree.
4. Writing is not owning: Bob's attempt to delete the desk fails with `Error::AccessDenied`.
5. After `revoke`, Bob is back to no access.

**CLI twin:** [`cps scope` and `cps access`](../cli/identity-scope-access.md).

Next: [encrypt your data](08-encryption.md)
