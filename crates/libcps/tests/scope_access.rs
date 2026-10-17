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
//! Live-node integration tests for the CPS Scope/Access API.
//!
//! These tests need a running Robonomics dev node with CPS pallet 1.1 with the CPS
//! view functions (runtime v52, metadata V16), so they are `#[ignore]`d by default:
//!
//! ```text
//! LIBCPS_TEST_WS=ws://127.0.0.1:9944 \
//!     cargo test -p libcps --test scope_access --no-default-features -- --ignored --test-threads=1
//! ```
//!
//! Effective permissions are always verified through
//! `Node::has_capability` (the CPS view function), never by reading
//! raw Access storage.

use libcps::crypto::{Signer, Sr25519};
use libcps::{Capability, Cps, CreateNode, Grant, NodeId};

fn ws_url() -> String {
    std::env::var("LIBCPS_TEST_WS").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string())
}

async fn connect() -> Cps {
    Cps::connect(Some(&ws_url()))
        .await
        .expect("dev node is reachable")
}

fn keypair(suri: &str) -> Signer<Sr25519> {
    Signer::from_suri(suri).expect("valid SURI")
}

async fn create(cps: &Cps, signer: &Signer<Sr25519>, parent: Option<NodeId>) -> NodeId {
    let params = CreateNode::new();
    let tx = match parent {
        Some(parent) => cps.node(parent).create_child(params),
        None => libcps::create_root(params),
    }
    .unwrap();
    cps.submit_finalized(&tx, signer).await.unwrap().result
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn queries_root_child_and_nested_scope() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let alice_id = alice.account_id();

    let root_tx = libcps::create_root(CreateNode::new().meta(b"root").payload(b"p0")).unwrap();
    let root = cps.submit_finalized(&root_tx, &alice).await.unwrap().result;
    let child = create(&cps, &alice, Some(root)).await;
    let nested = create(&cps, &alice, Some(child)).await;

    let root_info = cps.node(root).info().await.unwrap().unwrap();
    assert_eq!(root_info.parent, None);
    assert_eq!(cps.node(root).children().await.unwrap(), vec![child]);
    assert_eq!(cps.node(root).payload().await.unwrap().unwrap(), b"p0");
    assert_eq!(cps.node(root).meta().await.unwrap().unwrap(), b"root");

    let root_scope = cps.node(root).resolve_scope().await.unwrap();
    assert_eq!(root_scope.root, root);
    assert_eq!(root_scope.owner, alice_id);
    assert_eq!(root_info.scope, Some(root_scope.id));
    assert_eq!(
        cps.scope(root_scope.id)
            .info()
            .await
            .unwrap()
            .unwrap()
            .owner,
        alice_id
    );

    let child_info = cps.node(child).info().await.unwrap().unwrap();
    assert_eq!(child_info.parent, Some(root));
    assert_eq!(
        cps.node(child).resolve_scope().await.unwrap().id,
        root_scope.id
    );
    assert!(cps.node(child).payload().await.unwrap().is_none());

    // A nested Scope makes the node its own Scope root.
    let scope_id = cps
        .submit_finalized(&cps.node(nested).create_scope(), &alice)
        .await
        .unwrap()
        .result;
    let nested_scope = cps.node(nested).resolve_scope().await.unwrap();
    assert_eq!(nested_scope.id, scope_id);
    assert_eq!(nested_scope.root, nested);
    assert_ne!(nested_scope.id, root_scope.id);
    assert_eq!(nested_scope.owner, alice_id);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn missing_node_is_reported() {
    let cps = connect().await;
    let missing = NodeId::from(u64::MAX);

    assert!(cps.node(missing).info().await.unwrap().is_none());
    assert!(cps.node(missing).payload().await.unwrap().is_none());
    assert!(cps.node(missing).children().await.unwrap().is_empty());
    assert!(matches!(
        cps.node(missing).resolve_scope().await,
        Err(libcps::Error::NodeNotFound(id)) if id == missing
    ));
    assert!(matches!(
        cps.node(missing)
            .has_capability(&keypair("//Alice").account_id(), Capability::Write)
            .await,
        Err(libcps::Error::NodeNotFound(id)) if id == missing
    ));
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn set_and_clear_data() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let node = create(&cps, &alice, None).await;

    let receipt = cps
        .submit_finalized(&cps.node(node).set_payload(b"new").unwrap(), &alice)
        .await
        .unwrap();
    assert_ne!(receipt.block_hash, Default::default());
    assert_eq!(cps.node(node).payload().await.unwrap().unwrap(), b"new");

    cps.submit_finalized(&cps.node(node).clear_payload(), &alice)
        .await
        .unwrap();
    assert!(cps.node(node).payload().await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn historical_reads_are_block_consistent() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let alice_id = alice.account_id();
    let node = create(&cps, &alice, None).await;

    let first = cps
        .submit_finalized(&cps.node(node).set_payload(b"one").unwrap(), &alice)
        .await
        .unwrap()
        .block_hash;
    cps.submit_finalized(&cps.node(node).set_payload(b"two").unwrap(), &alice)
        .await
        .unwrap();

    // Every read through one `CpsAt` uses the same block.
    let at = cps.at(first).await.unwrap();
    assert_eq!(at.block_hash(), first);
    let pinned = at.node(node);
    assert_eq!(pinned.payload().await.unwrap().unwrap(), b"one");
    assert!(pinned.info().await.unwrap().is_some());
    assert!(pinned.children().await.unwrap().is_empty());

    let scope = pinned.resolve_scope().await.unwrap();
    assert_eq!(scope.root, node);
    assert_eq!(scope.owner, alice_id);
    assert!(at.scope(scope.id).info().await.unwrap().is_some());
    assert!(pinned
        .has_capability(&alice_id, Capability::Write)
        .await
        .unwrap());

    // The latest finalized block already has the second payload.
    let latest = cps.finalized().await.unwrap();
    assert_eq!(latest.node(node).payload().await.unwrap().unwrap(), b"two");
    assert_eq!(cps.node(node).payload().await.unwrap().unwrap(), b"two");
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn scope_replacement_yields_fresh_scope_id() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let node = create(&cps, &alice, None).await;

    let first = cps.node(node).resolve_scope().await.unwrap();
    cps.submit_finalized(&cps.node(node).replace_scope(), &alice)
        .await
        .unwrap();
    let second = cps.node(node).resolve_scope().await.unwrap();

    assert_eq!(second.root, first.root);
    assert_ne!(second.id, first.id);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn grants_in_node_and_subtree_modes_and_revoke() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let bob = keypair("//Bob").account_id();

    let root = create(&cps, &alice, None).await;
    let child = create(&cps, &alice, Some(root)).await;
    let can = |node: NodeId, capability| {
        let cps = &cps;
        async move {
            cps.node(node)
                .has_capability(&bob, capability)
                .await
                .unwrap()
        }
    };

    assert!(!can(root, Capability::Write).await);

    // Node mode covers only the granted node.
    cps.submit_finalized(
        &cps.node(root).grant(bob, Grant::write_node()).unwrap(),
        &alice,
    )
    .await
    .unwrap();
    assert!(can(root, Capability::Write).await);
    assert!(!can(child, Capability::Write).await);

    // Subtree mode also covers descendants in the same Scope.
    cps.submit_finalized(
        &cps.node(root).grant(bob, Grant::write_subtree()).unwrap(),
        &alice,
    )
    .await
    .unwrap();
    assert!(can(child, Capability::Write).await);

    // Capabilities are independent.
    assert!(!can(root, Capability::CreateScope).await);
    cps.submit_finalized(
        &cps.node(root)
            .grant(bob, Grant::create_scope_node())
            .unwrap(),
        &alice,
    )
    .await
    .unwrap();
    assert!(can(root, Capability::CreateScope).await);

    cps.submit_finalized(
        &cps.node(root).revoke(bob, Capability::Write).unwrap(),
        &alice,
    )
    .await
    .unwrap();
    assert!(!can(root, Capability::Write).await);
    assert!(!can(child, Capability::Write).await);
    assert!(can(root, Capability::CreateScope).await);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn subtree_grant_stops_at_nested_scope_boundary() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let bob = keypair("//Bob").account_id();

    let root = create(&cps, &alice, None).await;
    let nested = create(&cps, &alice, Some(root)).await;
    let below = create(&cps, &alice, Some(nested)).await;
    cps.submit_finalized(&cps.node(nested).create_scope(), &alice)
        .await
        .unwrap();

    cps.submit_finalized(
        &cps.node(root).grant(bob, Grant::write_subtree()).unwrap(),
        &alice,
    )
    .await
    .unwrap();

    let can = |node: NodeId| {
        let cps = &cps;
        async move {
            cps.node(node)
                .has_capability(&bob, Capability::Write)
                .await
                .unwrap()
        }
    };
    assert!(can(root).await);
    assert!(!can(nested).await);
    assert!(!can(below).await);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn unauthorized_write_is_denied() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let bob = keypair("//Bob");
    let node = create(&cps, &alice, None).await;

    let err = cps
        .submit_finalized(&cps.node(node).set_payload(b"x").unwrap(), &bob)
        .await
        .unwrap_err();
    assert!(matches!(err, libcps::Error::AccessDenied), "{err:?}");
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn deletes_leaf_ordinary_node_and_leaf_scope_root() {
    let cps = connect().await;
    let alice = keypair("//Alice");

    let root = create(&cps, &alice, None).await;
    let ordinary = create(&cps, &alice, Some(root)).await;
    let scoped = create(&cps, &alice, Some(root)).await;
    cps.submit_finalized(&cps.node(scoped).create_scope(), &alice)
        .await
        .unwrap();

    cps.submit_finalized(&cps.node(ordinary).delete(), &alice)
        .await
        .unwrap();
    cps.submit_finalized(&cps.node(scoped).delete(), &alice)
        .await
        .unwrap();

    assert!(cps.node(ordinary).info().await.unwrap().is_none());
    assert!(cps.node(scoped).info().await.unwrap().is_none());
    assert!(cps.node(root).children().await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn create_scope_refuses_existing_scope_root() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let root = create(&cps, &alice, None).await;
    let before = cps.node(root).resolve_scope().await.unwrap();

    let err = cps
        .submit_finalized(&cps.node(root).create_scope(), &alice)
        .await
        .unwrap_err();
    assert!(matches!(err, libcps::Error::ScopeAlreadyExists(id) if id == root));
    assert_eq!(cps.node(root).resolve_scope().await.unwrap().id, before.id);

    // A node that is not a scope root can still get one.
    let child = create(&cps, &alice, Some(root)).await;
    cps.submit_finalized(&cps.node(child).create_scope(), &alice)
        .await
        .unwrap();
}
