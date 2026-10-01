//! Live-node integration tests for the CPS Scope/Access API.
//!
//! These tests need a running Robonomics dev node with CPS pallet 1.0 and the
//! `CpsApi` runtime API, so they are `#[ignore]`d by default:
//!
//! ```text
//! LIBCPS_TEST_WS=ws://127.0.0.1:9944 \
//!     cargo test -p libcps --test scope_access --no-default-features -- --ignored --test-threads=1
//! ```
//!
//! Effective permissions are always verified through
//! `NodeAccess::has_capability` (the `CpsApi` runtime API), never by reading
//! raw Access storage.

use libcps::crypto::{Signer, Sr25519};
use libcps::prelude::*;
use libcps::{Access, Capability, Client, CreateNode, NodeId};

fn ws_url() -> String {
    std::env::var("LIBCPS_TEST_WS").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string())
}

async fn connect() -> Client {
    Client::connect(Some(&ws_url()))
        .await
        .expect("dev node is reachable")
}

fn keypair(suri: &str) -> Signer<Sr25519> {
    Signer::from_suri(suri).expect("valid SURI")
}

fn data(bytes: &str) -> Option<Vec<u8>> {
    Some(bytes.as_bytes().to_vec())
}

async fn create(client: &Client, signer: &Signer<Sr25519>, parent: Option<NodeId>) -> NodeId {
    let params = CreateNode::default();
    let tx = match parent {
        Some(parent) => parent.create_child(params),
        None => libcps::create_root(params),
    }
    .unwrap();
    client.submit_finalized(&tx, signer).await.unwrap().result
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn queries_root_child_and_nested_scope() {
    let client = connect().await;
    let alice = keypair("//Alice");
    let alice_id = alice.account_id();

    let root_tx = libcps::create_root(CreateNode {
        meta: data("root"),
        payload: data("p0"),
    })
    .unwrap();
    let root = client
        .submit_finalized(&root_tx, &alice)
        .await
        .unwrap()
        .result;
    let child = create(&client, &alice, Some(root)).await;
    let nested = create(&client, &alice, Some(child)).await;

    let root_info = root.info(&client).await.unwrap().unwrap();
    assert_eq!(root_info.parent, None);
    assert_eq!(root.children(&client).await.unwrap(), vec![child]);
    assert_eq!(root.payload(&client).await.unwrap().unwrap(), b"p0");
    assert_eq!(root.meta(&client).await.unwrap().unwrap(), b"root");

    let root_scope = root.resolve_scope(&client).await.unwrap();
    assert_eq!(root_scope.root, root);
    assert_eq!(root_scope.owner, alice_id);
    assert_eq!(root_info.scope, Some(root_scope.id));
    assert_eq!(
        root_scope.id.info(&client).await.unwrap().unwrap().owner,
        alice_id
    );

    let child_info = child.info(&client).await.unwrap().unwrap();
    assert_eq!(child_info.parent, Some(root));
    assert_eq!(
        child.resolve_scope(&client).await.unwrap().id,
        root_scope.id
    );
    assert!(child.payload(&client).await.unwrap().is_none());

    // A nested Scope makes the node its own Scope root.
    let scope_id = client
        .submit_finalized(&nested.create_scope(), &alice)
        .await
        .unwrap()
        .result;
    let nested_scope = nested.resolve_scope(&client).await.unwrap();
    assert_eq!(nested_scope.id, scope_id);
    assert_eq!(nested_scope.root, nested);
    assert_ne!(nested_scope.id, root_scope.id);
    assert_eq!(nested_scope.owner, alice_id);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn missing_node_is_reported() {
    let client = connect().await;
    let missing = NodeId::from(u64::MAX);

    assert!(missing.info(&client).await.unwrap().is_none());
    assert!(missing.payload(&client).await.unwrap().is_none());
    assert!(missing.children(&client).await.unwrap().is_empty());
    assert!(matches!(
        missing.resolve_scope(&client).await,
        Err(libcps::Error::NodeNotFound(id)) if id == missing
    ));
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn set_and_clear_data() {
    let client = connect().await;
    let alice = keypair("//Alice");
    let node = create(&client, &alice, None).await;

    let receipt = client
        .submit_finalized(&node.set_payload(b"new").unwrap(), &alice)
        .await
        .unwrap();
    assert_ne!(receipt.block_hash, Default::default());
    assert_eq!(node.payload(&client).await.unwrap().unwrap(), b"new");

    client
        .submit_finalized(&node.clear_payload(), &alice)
        .await
        .unwrap();
    assert!(node.payload(&client).await.unwrap().is_none());
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn scope_replacement_yields_fresh_scope_id() {
    let client = connect().await;
    let alice = keypair("//Alice");
    let node = create(&client, &alice, None).await;

    let first = node.resolve_scope(&client).await.unwrap();
    client
        .submit_finalized(&node.create_scope(), &alice)
        .await
        .unwrap();
    let second = node.resolve_scope(&client).await.unwrap();

    assert_eq!(second.root, first.root);
    assert_ne!(second.id, first.id);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn grants_in_node_and_subtree_modes_and_revoke() {
    let client = connect().await;
    let alice = keypair("//Alice");
    let bob = keypair("//Bob").account_id();

    let root = create(&client, &alice, None).await;
    let child = create(&client, &alice, Some(root)).await;
    let can = |node: NodeId, capability| {
        let client = &client;
        async move { node.has_capability(client, &bob, capability).await.unwrap() }
    };

    assert!(!can(root, Capability::Write).await);

    // Node mode covers only the granted node.
    client
        .submit_finalized(&root.grant(bob, Access::write_node()).unwrap(), &alice)
        .await
        .unwrap();
    assert!(can(root, Capability::Write).await);
    assert!(!can(child, Capability::Write).await);

    // Subtree mode also covers descendants in the same Scope.
    client
        .submit_finalized(&root.grant(bob, Access::write_subtree()).unwrap(), &alice)
        .await
        .unwrap();
    assert!(can(child, Capability::Write).await);

    // Capabilities are independent.
    assert!(!can(root, Capability::CreateScope).await);
    client
        .submit_finalized(
            &root.grant(bob, Access::create_scope_node()).unwrap(),
            &alice,
        )
        .await
        .unwrap();
    assert!(can(root, Capability::CreateScope).await);

    client
        .submit_finalized(&root.revoke(bob, Capability::Write).unwrap(), &alice)
        .await
        .unwrap();
    assert!(!can(root, Capability::Write).await);
    assert!(!can(child, Capability::Write).await);
    assert!(can(root, Capability::CreateScope).await);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn subtree_grant_stops_at_nested_scope_boundary() {
    let client = connect().await;
    let alice = keypair("//Alice");
    let bob = keypair("//Bob").account_id();

    let root = create(&client, &alice, None).await;
    let nested = create(&client, &alice, Some(root)).await;
    let below = create(&client, &alice, Some(nested)).await;
    client
        .submit_finalized(&nested.create_scope(), &alice)
        .await
        .unwrap();

    client
        .submit_finalized(&root.grant(bob, Access::write_subtree()).unwrap(), &alice)
        .await
        .unwrap();

    let can = |node: NodeId| {
        let client = &client;
        async move {
            node.has_capability(client, &bob, Capability::Write)
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
    let client = connect().await;
    let alice = keypair("//Alice");
    let bob = keypair("//Bob");
    let node = create(&client, &alice, None).await;

    let err = client
        .submit_finalized(&node.set_payload(b"x").unwrap(), &bob)
        .await
        .unwrap_err();
    assert!(matches!(err, libcps::Error::AccessDenied), "{err:?}");
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn deletes_leaf_ordinary_node_and_leaf_scope_root() {
    let client = connect().await;
    let alice = keypair("//Alice");

    let root = create(&client, &alice, None).await;
    let ordinary = create(&client, &alice, Some(root)).await;
    let scoped = create(&client, &alice, Some(root)).await;
    client
        .submit_finalized(&scoped.create_scope(), &alice)
        .await
        .unwrap();

    client
        .submit_finalized(&ordinary.delete(), &alice)
        .await
        .unwrap();
    client
        .submit_finalized(&scoped.delete(), &alice)
        .await
        .unwrap();

    assert!(ordinary.info(&client).await.unwrap().is_none());
    assert!(scoped.info(&client).await.unwrap().is_none());
    assert!(root.children(&client).await.unwrap().is_empty());
}
