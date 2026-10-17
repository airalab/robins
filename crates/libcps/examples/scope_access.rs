///////////////////////////////////////////////////////////////////////////////
//
//  Copyright 2018-2026 Robonomics Network <research@robonomics.network>
//
//  Licensed under the Apache License, Version 2.0 (the "License");
//  you may not use this file except in compliance with the License.
//  You may obtain a copy of the License at
//
//      http://www.apache.org/licenses/LICENSE-2.0
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
//
//! Scopes, grants and authorization on a local testnet.
//!
//! Ownership belongs to a *scope*, not to a node. The example:
//!
//! 1. creates a tree and shows the scope its nodes resolve to;
//! 2. makes a nested scope with `create_scope`, shows that `create_scope` refuses a
//!    node that already roots a scope and uses `replace_scope` to replace it
//!    explicitly;
//! 3. grants `//Bob` capabilities in `Node` and `Subtree` mode and asks the runtime
//!    what he may do (the library never decides authorization itself);
//! 4. lets Bob write, and shows the typed error when he is not allowed to.
//!
//! Start a development node first (see `rpc_client_write`), then:
//!
//! ```text
//! cargo run -p libcps --example scope_access --no-default-features
//! ```
//!
//! Environment: `ROBONOMICS_WS_URL` (default `ws://127.0.0.1:9944`) and
//! `ROBONOMICS_SURI` (default `//Alice`, the scope owner).

use libcps::{
    crypto::{Signer, Sr25519},
    Capability, Cps, CreateNode, Error, Grant, NodeId,
};

/// Create a node under `parent` (or a root node) and return its id.
async fn create(
    cps: &Cps,
    signer: &Signer<Sr25519>,
    parent: Option<NodeId>,
) -> Result<NodeId, Error> {
    let params = CreateNode::new();
    let tx = match parent {
        Some(parent) => cps.node(parent).create_child(params)?,
        None => libcps::create_root(params)?,
    };
    Ok(cps.submit_finalized(&tx, signer).await?.result)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url =
        std::env::var("ROBONOMICS_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string());
    let suri = std::env::var("ROBONOMICS_SURI").unwrap_or_else(|_| "//Alice".to_string());
    let owner = Signer::<Sr25519>::from_suri(&suri)?;
    let bob = Signer::<Sr25519>::from_suri("//Bob")?;
    let cps = Cps::connect_rpc(&url).await?;

    let root = create(&cps, &owner, None).await?;
    let room = create(&cps, &owner, Some(root)).await?;
    let desk = create(&cps, &owner, Some(room)).await?;

    // Every node resolves to the scope of its nearest ancestor that has one.
    let scope = cps.node(desk).resolve_scope().await?;
    println!(
        "desk {desk} resolves to scope {} rooted at {} (owner {}), path {:?}",
        scope.id, scope.root, scope.owner, scope.path
    );
    let record = cps.scope(scope.id).info().await?.expect("scope exists");
    println!("scope record: {} access entries", record.access_count);

    // A nested scope; `create_scope` is only for nodes that do not root a scope yet.
    let nested = cps
        .submit_finalized(&cps.node(room).create_scope(), &owner)
        .await?
        .result;
    println!("room {room} now roots scope {nested}");
    match cps
        .submit_finalized(&cps.node(room).create_scope(), &owner)
        .await
    {
        Err(Error::ScopeAlreadyExists(id)) => {
            println!("create_scope refused: {id} already roots a scope")
        }
        other => println!("unexpected: {:?}", other.map(|r| r.result)),
    }
    // Replacing is allowed, but must be asked for explicitly: it drops access entries.
    let replaced = cps
        .submit_finalized(&cps.node(room).replace_scope(), &owner)
        .await?
        .result;
    println!("room {room}: scope {nested} replaced by {replaced}");

    // Bob has no rights yet; the runtime is the source of truth.
    let bob_id = bob.account_id();
    let can = |node: NodeId, capability| {
        let cps = &cps;
        async move { cps.node(node).has_capability(&bob_id, capability).await }
    };
    println!(
        "bob can write room: {}",
        can(room, Capability::Write).await?
    );

    // Node mode covers one node, Subtree mode also its descendants in the scope.
    cps.submit_finalized(&cps.node(room).grant(bob_id, Grant::write_node())?, &owner)
        .await?;
    println!(
        "after write_node:    room {}, desk {}",
        can(room, Capability::Write).await?,
        can(desk, Capability::Write).await?
    );
    cps.submit_finalized(
        &cps.node(room).grant(bob_id, Grant::write_subtree())?,
        &owner,
    )
    .await?;
    println!(
        "after write_subtree: room {}, desk {}",
        can(room, Capability::Write).await?,
        can(desk, Capability::Write).await?
    );

    // Bob may now write the desk, but not delete it (only the scope owner may).
    cps.submit_finalized(&cps.node(desk).set_payload(b"bob was here")?, &bob)
        .await?;
    match cps.submit_finalized(&cps.node(desk).delete(), &bob).await {
        Err(Error::AccessDenied) => println!("bob cannot delete: access denied"),
        other => println!("unexpected: {:?}", other.map(|r| r.result)),
    }

    // The account-addressed view answers the same question for nodes with identity.
    // `address_capability` needs the target to be bound to an account, so it fails
    // with `IdentityNotFound` for the desk, which has no identity.
    let target = Signer::<Sr25519>::from_suri("//Desk")?.account_id();
    match cps
        .address(target)
        .has_capability(&bob_id, Capability::Write)
        .await
    {
        Err(Error::IdentityNotFound(a)) => println!("{a} is not an identity: no answer"),
        other => println!("unexpected: {other:?}"),
    }

    cps.submit_finalized(&cps.node(room).revoke(bob_id, Capability::Write)?, &owner)
        .await?;
    println!("after revoke: room {}", can(room, Capability::Write).await?);

    // Clean up: leaves first.
    for node in [desk, room, root] {
        cps.submit_finalized(&cps.node(node).delete(), &owner)
            .await?;
    }
    Ok(())
}
