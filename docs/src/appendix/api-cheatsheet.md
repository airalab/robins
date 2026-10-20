# B. API cheat sheet

Ordered like the book, from the most used to the most advanced.

| Task | Call |
| --- | --- |
| Run a light client | `LightNode::start(LightNodeConfig::default())`, `light.wait_ready()`, `light.rpc_url()` |
| Connect (RPC) | `Cps::connect(url)` |
| Read a node | `cps.node(id).info() / meta() / payload() / children() / parent()` |
| Consistent multi-read | `let at = cps.finalized().await?` (or `cps.at(hash)`), then `at.node(id)...` |
| Follow changes | `node.watch_payload()`, `node.watch_meta()`, then `watcher.next()` |
| Build a transaction | `libcps::create_root(CreateNode::new()...)`, `node.create_child(..)`, `set_meta`, `set_payload`, `clear_*`, `delete` |
| Send | `cps.submit_finalized(&tx, &signer)`, or `cps.submit(&tx, &signer)` + `wait_finalized()` |
| Identity | `CreateNode::identity(..)`, `node.set_identity(..)`, `clear_identity()`, `node.identity()` |
| By account | `cps.address(account).node_id() / parent() / children() / payload() / scope_id()` |
| Missing vs unset | `node.snapshot()` |
| Scope | `node.resolve_scope()`, `cps.scope(id).info()`, `node.create_scope()` |
| Access | `node.grant(account, Grant::..)`, `revoke(account, cap)`, `has_capability(&account, cap)` |
| Encrypt | `signer.encrypt(data, &receiver_public, algorithm)`, `receiver.decrypt(&msg, Some(&sender_public))` |

## Errors worth handling

| Error | Meaning |
| --- | --- |
| `Error::NodeNotFound` | the node does not exist at the block read |
| `Error::PayloadTooLarge` / `MetaTooLarge` | value exceeds the runtime limit; caught while building |
| `Error::AccessDenied` | the runtime refused: the signer lacks the capability |
| `Error::ScopeAlreadyExists` | `create_scope` on a node that already roots a scope |
| `Error::IdentityNotFound` | the account is the identity of no node |
| `Error::MissingProjectedIdentity` | an address-projected read met a node without identity |

## Example index

| # | Example | Needs |
| --- | --- | --- |
| 01 | `01_light_client` | internet |
| 02 | `02_explore_network` | internet |
| 03 | `03_watch` | internet |
| 04 | `04_offline_transactions` | nothing |
| 05 | `05_local_node_tree` | local node |
| 06 | `06_identity` | local node |
| 07 | `07_scope_access` | local node |
| 08 | `08_encryption` | nothing |
