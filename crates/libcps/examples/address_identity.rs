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
///////////////////////////////////////////////////////////////////////////////
//! Address-based access and node identities on a local testnet.
//!
//! CPS has one tree with two ways to address it: the canonical [`NodeId`] and the
//! identity [`AccountId`] a node may be bound to. This example creates a small tree,
//! binds the signer as the identity of its root and walks it through both
//! interfaces, converts explicitly between them and reads a consistent
//! [`NodeSnapshot`]. It finally clears the identity and removes the nodes.
//!
//! Start a development node first (see `rpc_client_write`), then:
//!
//! ```text
//! cargo run -p libcps --example address_identity --no-default-features
//! ```
//!
//! Environment: `ROBONOMICS_WS_URL` (default `ws://127.0.0.1:9944`) and
//! `ROBONOMICS_SURI` (default `//Alice`).

use libcps::{
    crypto::{Signer, Sr25519},
    Cps, CreateNode, Error, IdentityAssignment, NodeId,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url =
        std::env::var("ROBONOMICS_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string());
    let suri = std::env::var("ROBONOMICS_SURI").unwrap_or_else(|_| "//Alice".to_string());
    let signer = Signer::<Sr25519>::from_suri(&suri)?;
    let account = signer.account_id();
    let cps = Cps::connect_rpc(&url).await?;

    // A root node can only get the signer itself as identity; no permit is needed.
    let root = cps
        .submit_finalized(
            &libcps::create_root(
                CreateNode::new()
                    .meta(b"gateway")
                    .payload(b"online")
                    .identity(IdentityAssignment::new(account)),
            )?,
            &signer,
        )
        .await?
        .result;
    let child = cps
        .submit_finalized(
            &cps.node(root)
                .create_child(CreateNode::new().payload(b"sensor"))?,
            &signer,
        )
        .await?
        .result;
    println!("root {root} (identity {account}), child {child} (no identity)");

    // Explicit conversion between the two address spaces; both are fallible.
    let identity = cps.node(root).identity().await?;
    let node_id = cps.address(account).node_id().await?;
    println!("node {root} -> identity {identity:?}; account -> node {node_id:?}");
    assert_eq!(identity, Some(account));
    assert_eq!(node_id, Some(root));

    // Node interface: results are NodeIds.
    println!(
        "children by node id: {:?}",
        cps.node(root).children().await?
    );

    // Address interface: results are accounts. The child has no identity, so the
    // runtime refuses to project it and the whole call fails (nothing is omitted).
    match cps.address(account).children().await {
        Err(Error::MissingProjectedIdentity) => {
            println!("children by account: a child has no identity, projection refused")
        }
        other => println!("children by account: {other:?}"),
    }
    println!(
        "payload by account: {:?}",
        cps.address(account)
            .payload()
            .await?
            .map(|p| String::from_utf8_lossy(&p).into_owned())
    );

    // An account bound to no node is reported, not silently empty.
    let stranger = Signer::<Sr25519>::from_suri("//Stranger")?.account_id();
    println!(
        "stranger -> node {:?}",
        cps.address(stranger).node_id().await?
    );
    match cps.address(stranger).payload().await {
        Err(Error::IdentityNotFound(a)) => println!("{a} is the identity of no node"),
        other => println!("unexpected: {other:?}"),
    }

    // Several reads at one block: `CpsAt` pins them all to the same block hash.
    let at = cps.finalized().await?;
    println!("pinned reads at block {:?}", at.block_hash());
    let scope = at.node(root).resolve_scope().await?;
    let by_account_scope = at.address(account).scope_id().await?;
    assert_eq!(scope.id, by_account_scope);
    println!("scope {} owned by {}", scope.id, scope.owner);

    // A snapshot tells a missing node apart from one whose fields are unset.
    let snapshot = at.node(child).snapshot().await?.expect("child exists");
    println!(
        "child snapshot: parent {:?}, identity {:?}, meta {:?}, payload {:?}",
        snapshot.parent, snapshot.identity, snapshot.meta, snapshot.payload
    );
    assert!(at.node(NodeId::from(u64::MAX)).snapshot().await?.is_none());

    // Identity changes are explicit transactions.
    cps.submit_finalized(&cps.node(root).clear_identity(), &signer)
        .await?;
    println!("identity cleared: {:?}", cps.node(root).identity().await?);

    cps.submit_finalized(&cps.node(child).delete(), &signer)
        .await?;
    cps.submit_finalized(&cps.node(root).delete(), &signer)
        .await?;
    println!("deleted {child} and {root}");
    Ok(())
}
