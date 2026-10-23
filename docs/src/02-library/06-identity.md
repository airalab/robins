# Identities and addresses

> **Example:** `06_identity` &nbsp;·&nbsp; **Needs:** a local development node &nbsp;·&nbsp; **API:** `IdentityAssignment`, `node.identity()`, `Cps::address`, `NodeSnapshot`

Node ids are numbers handed out by the chain. Devices, however, already have a natural name: their
account. CPS 1.1 lets a node be bound to an **identity account**, so a sensor can say "my data lives
at node bound to *me*" without anybody having to tell it a node id first.

```bash
cargo run -p libcps --example 06_identity --no-default-features
```

```rust,ignore
{{#include ../../../crates/libcps/examples/06_identity.rs:code}}
```

## Two ways to address one tree

| Interface | Handle | Results are |
| --- | --- | --- |
| Canonical | `cps.node(NodeId)` | `NodeId`s |
| By identity | `cps.address(AccountId)` | accounts, read-only |

There is one tree; these are two projections of it. Convert explicitly with `node.identity()` (node
→ account) and `address.node_id()` (account → node); both are fallible and return `None` when there
is no binding.

The address projection is strict. If a child has no identity the runtime cannot express it as an
account, so `cps.address(account).children()` fails with `Error::MissingProjectedIdentity` instead
of silently omitting the child. An account that is the identity of no node yields
`Error::IdentityNotFound`.

## Permits: identities for other accounts

The signer's own account can become the identity of a **root** node directly, as in the example.
To bind another account (a device that holds its own key) to a node in your scope, that account signs
an **identity permit** (`IdentityPermit`) for the scope. `libcps` exposes
`cps.identity_permit_payload(..)` for the exact bytes to sign and `permit::PermitArtifact` for the JSON
artifact that is handed over; the node owner then submits `IdentityAssignment::with_permit(..)`.
The CLI wraps the whole flow in [`cps identity permit`](../01-cli/06-identity-scope-access.md).

## Snapshots

`at.node(id).snapshot()` reads parent, scope, identity, metadata and payload at one block and tells a
**missing** node (`None`) apart from a node whose fields are merely unset.

**CLI twin:** [`cps identity`](../01-cli/06-identity-scope-access.md).

Next: [let someone else write](07-scopes-and-access.md)
