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
//! Effective permissions are always verified through `Node::has_capability`
//! (the `CpsApi` runtime API), never by reading raw Access storage.

use libcps::blockchain::{BoundedVec, Client, Config};
use libcps::node::{Capability, GrantMode, Node, NodeId};
use subxt::utils::AccountId32;

fn ws_url() -> String {
    std::env::var("LIBCPS_TEST_WS").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string())
}

async fn connect(suri: &str) -> Client {
    Client::new(&Config {
        ws_url: ws_url(),
        suri: Some(suri.to_string()),
    })
    .await
    .expect("dev node is reachable")
}

fn account(client: &Client) -> AccountId32 {
    client
        .require_keypair()
        .expect("client has a keypair")
        .public_key()
        .to_account_id()
}

fn data(bytes: &str) -> Option<BoundedVec<u8>> {
    Some(BoundedVec(bytes.as_bytes().to_vec()))
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn queries_root_child_and_nested_scope() {
    let alice = connect("//Alice").await;
    let alice_id = account(&alice);

    let root = Node::create(&alice, None, data("root"), data("p0"))
        .await
        .unwrap();
    let child = Node::create(&alice, Some(root.id()), data("child"), None)
        .await
        .unwrap();
    let nested = Node::create(&alice, Some(child.id()), None, None)
        .await
        .unwrap();

    let root_info = root.query().await.unwrap();
    assert_eq!(root_info.parent, None);
    assert_eq!(root_info.children, vec![child.id()]);
    assert_eq!(root_info.scope.root, root.id());
    assert_eq!(root_info.scope.owner, alice_id);
    assert_eq!(root_info.payload.unwrap().0, b"p0");

    let child_info = child.query().await.unwrap();
    assert_eq!(child_info.parent, Some(root.id()));
    assert_eq!(child_info.scope.id, root_info.scope.id);
    assert!(child_info.payload.is_none());

    // A nested Scope makes the node its own Scope root.
    nested.create_scope().await.unwrap();
    let nested_scope = nested.resolve_scope().await.unwrap();
    assert_eq!(nested_scope.root, nested.id());
    assert_ne!(nested_scope.id, root_info.scope.id);
    assert_eq!(nested_scope.owner, alice_id);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn query_at_reads_historical_state() {
    let alice = connect("//Alice").await;
    let node = Node::create(&alice, None, data("m"), data("old"))
        .await
        .unwrap();

    let before = alice.api.at_current_block().await.unwrap().block_hash();
    node.set_payload(data("new")).await.unwrap();

    let old = node.query_at(before).await.unwrap();
    let latest = node.query().await.unwrap();
    assert_eq!(old.payload.unwrap().0, b"old");
    assert_eq!(latest.payload.unwrap().0, b"new");
    assert_eq!(old.scope, latest.scope);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn scope_replacement_yields_fresh_scope_id() {
    let alice = connect("//Alice").await;
    let node = Node::create(&alice, None, None, None).await.unwrap();

    let first = node.resolve_scope().await.unwrap();
    node.create_scope().await.unwrap();
    let second = node.resolve_scope().await.unwrap();

    assert_eq!(second.root, first.root);
    assert_ne!(second.id, first.id);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn grants_in_node_and_subtree_modes_and_revoke() {
    let alice = connect("//Alice").await;
    let bob = account(&connect("//Bob").await);

    let root = Node::create(&alice, None, None, None).await.unwrap();
    let child = Node::create(&alice, Some(root.id()), None, None)
        .await
        .unwrap();

    assert!(!root.has_capability(bob, Capability::Write).await.unwrap());

    // Node mode covers only the granted node.
    root.grant_access(bob, Capability::Write, GrantMode::Node)
        .await
        .unwrap();
    assert!(root.has_capability(bob, Capability::Write).await.unwrap());
    assert!(!child.has_capability(bob, Capability::Write).await.unwrap());

    // Subtree mode also covers descendants in the same Scope.
    root.grant_access(bob, Capability::Write, GrantMode::Subtree)
        .await
        .unwrap();
    assert!(child.has_capability(bob, Capability::Write).await.unwrap());

    // Capabilities are independent.
    assert!(!root
        .has_capability(bob, Capability::CreateScope)
        .await
        .unwrap());
    root.grant_access(bob, Capability::CreateScope, GrantMode::Node)
        .await
        .unwrap();
    assert!(root
        .has_capability(bob, Capability::CreateScope)
        .await
        .unwrap());

    root.revoke_access(bob, Capability::Write).await.unwrap();
    assert!(!root.has_capability(bob, Capability::Write).await.unwrap());
    assert!(!child.has_capability(bob, Capability::Write).await.unwrap());
    assert!(root
        .has_capability(bob, Capability::CreateScope)
        .await
        .unwrap());
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn subtree_grant_stops_at_nested_scope_boundary() {
    let alice = connect("//Alice").await;
    let bob = account(&connect("//Bob").await);

    let root = Node::create(&alice, None, None, None).await.unwrap();
    let nested = Node::create(&alice, Some(root.id()), None, None)
        .await
        .unwrap();
    let below = Node::create(&alice, Some(nested.id()), None, None)
        .await
        .unwrap();
    nested.create_scope().await.unwrap();

    root.grant_access(bob, Capability::Write, GrantMode::Subtree)
        .await
        .unwrap();

    assert!(root.has_capability(bob, Capability::Write).await.unwrap());
    assert!(!nested.has_capability(bob, Capability::Write).await.unwrap());
    assert!(!below.has_capability(bob, Capability::Write).await.unwrap());
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn deletes_leaf_ordinary_node_and_leaf_scope_root() {
    let alice = connect("//Alice").await;

    let root = Node::create(&alice, None, None, None).await.unwrap();
    let ordinary = Node::create(&alice, Some(root.id()), None, None)
        .await
        .unwrap();
    let scoped = Node::create(&alice, Some(root.id()), None, None)
        .await
        .unwrap();
    scoped.create_scope().await.unwrap();

    let ordinary_id: NodeId = ordinary.id();
    let scoped_id: NodeId = scoped.id();
    ordinary.delete().await.unwrap();
    scoped.delete().await.unwrap();

    assert!(Node::new(&alice, ordinary_id).query().await.is_err());
    assert!(Node::new(&alice, scoped_id).query().await.is_err());
    assert!(root.query().await.unwrap().children.is_empty());
}
