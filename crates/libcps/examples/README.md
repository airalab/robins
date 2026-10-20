# libcps examples

Numbered runnable examples that walk through the `libcps` API from the simplest to the most
advanced. They are the code of the **Library** part of the documentation book (`docs/` in the
repository root), where each one is explained as a stop on a journey: first explore the Robonomics
Network, then launch your own.

| # | Example | Needs | You learn |
| --- | --- | --- | --- |
| 01 | `01_light_client` | internet | embedded light client, connection status |
| 02 | `02_explore_network` | internet | read the live network (sensor.social), pinned reads |
| 03 | `03_watch` | internet | follow a live node with watchers |
| 04 | `04_offline_transactions` | nothing | build `Transaction<T>` offline, limits |
| 05 | `05_local_node_tree` | local node | create, update, delete, submit |
| 06 | `06_identity` | local node | identities, address-based access |
| 07 | `07_scope_access` | local node | scopes, grants, writing as another account |
| 08 | `08_encryption` | nothing | `Signer`, `Cipher`, encrypted payloads |

Run from the repository root (inside `nix develop`):

```bash
cargo run -p libcps --example 04_offline_transactions --no-default-features
cargo run -p libcps --example 02_explore_network --no-default-features --features light-client -- <ROOT_NODE_ID>
```

Examples 01-03 need the `light-client` feature. Examples 05-07 write to a chain: start a local
development node first (a Docker one-liner, see the book's *Launch your own node* chapter) and, if
needed, set `ROBONOMICS_WS_URL` (default `ws://127.0.0.1:9944`) and `ROBONOMICS_SURI` (default
`//Alice`).

[`shell/`](shell) holds the CLI counterparts: `cli_journey.sh` and the appendix script
`09_profiles.sh`.

## Read the book

```bash
nix develop -c mdbook serve docs --open
```
