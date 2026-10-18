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
//! Live-node integration tests for CPS 1.1 node identity and address views.
//!
//! They need a running Robonomics dev node (runtime v52), so they are
//! `#[ignore]`d by default:
//!
//! ```text
//! LIBCPS_TEST_WS=ws://127.0.0.1:9944 \
//!     cargo test -p libcps --test identity_address --no-default-features -- --ignored --test-threads=1
//! ```
//!
//! Identities are unique per chain, so each test deletes the nodes it created.

use libcps::crypto::{Signer, Sr25519};
use libcps::{Cps, CreateNode, Error, IdentityAssignment, IdentityError, NodeId};

fn ws_url() -> String {
    std::env::var("LIBCPS_TEST_WS").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string())
}

async fn connect() -> Cps {
    Cps::connect(&ws_url())
        .await
        .expect("dev node is reachable")
}

fn keypair(suri: &str) -> Signer<Sr25519> {
    Signer::from_suri(suri).expect("valid SURI")
}

/// Create a root node whose identity is the signer itself.
async fn create_identified_root(cps: &Cps, signer: &Signer<Sr25519>) -> NodeId {
    let params = CreateNode::new()
        .meta(b"root")
        .payload(b"p0")
        .identity(IdentityAssignment::new(signer.account_id()));
    let tx = libcps::create_root(params).unwrap();
    cps.submit_finalized(&tx, signer).await.unwrap().result
}

async fn delete(cps: &Cps, signer: &Signer<Sr25519>, node: NodeId) {
    cps.submit_finalized(&cps.node(node).delete(), signer)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn identity_assignment_lookup_and_conversion() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let account = alice.account_id();
    let root = create_identified_root(&cps, &alice).await;

    // NodeId -> AccountId and back, both explicit.
    assert_eq!(cps.node(root).identity().await.unwrap(), Some(account));
    assert_eq!(cps.address(account).node_id().await.unwrap(), Some(root));

    // The snapshot reports identity, data and the topology at one block.
    let snapshot = cps.node(root).snapshot().await.unwrap().unwrap();
    assert_eq!(snapshot.id, root);
    assert_eq!(snapshot.identity, Some(account));
    assert_eq!(snapshot.meta.as_deref(), Some(&b"root"[..]));
    assert_eq!(snapshot.payload.as_deref(), Some(&b"p0"[..]));
    assert_eq!(snapshot.parent, None);

    // The address interface reads the same node and stays in the account space.
    assert_eq!(
        cps.address(account).payload().await.unwrap().as_deref(),
        Some(&b"p0"[..])
    );
    assert_eq!(cps.address(account).parent().await.unwrap(), None);
    assert!(cps.address(account).children().await.unwrap().is_empty());

    delete(&cps, &alice, root).await;
    assert_eq!(cps.address(account).node_id().await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn identity_can_be_cleared_and_set_again() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let account = alice.account_id();
    let root = create_identified_root(&cps, &alice).await;

    cps.submit_finalized(&cps.node(root).clear_identity(), &alice)
        .await
        .unwrap();
    assert_eq!(cps.node(root).identity().await.unwrap(), None);
    assert!(matches!(
        cps.address(account).payload().await,
        Err(Error::IdentityNotFound(a)) if a == account
    ));

    // Clearing again is rejected by the runtime.
    let err = cps
        .submit_finalized(&cps.node(root).clear_identity(), &alice)
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::Identity(IdentityError::NotSet)),
        "{err:?}"
    );

    let tx = cps
        .node(root)
        .set_identity(IdentityAssignment::new(account))
        .unwrap();
    cps.submit_finalized(&tx, &alice).await.unwrap();
    assert_eq!(cps.node(root).identity().await.unwrap(), Some(account));

    // The same account cannot identify a second node.
    let other = cps
        .submit_finalized(&libcps::create_root(CreateNode::new()).unwrap(), &alice)
        .await
        .unwrap()
        .result;
    let tx = cps
        .node(other)
        .set_identity(IdentityAssignment::new(account))
        .unwrap();
    let err = cps.submit_finalized(&tx, &alice).await.unwrap_err();
    assert!(
        matches!(err, Error::Identity(IdentityError::AlreadyInUse)),
        "{err:?}"
    );

    delete(&cps, &alice, other).await;
    delete(&cps, &alice, root).await;
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn address_views_stay_in_the_account_space() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let account = alice.account_id();
    let root = create_identified_root(&cps, &alice).await;
    let child = cps
        .submit_finalized(
            &cps.node(root).create_child(CreateNode::new()).unwrap(),
            &alice,
        )
        .await
        .unwrap()
        .result;

    // NodeId space: the child is visible. Account space: it has no identity, so
    // the projection fails as a whole instead of hiding it.
    assert_eq!(cps.node(root).children().await.unwrap(), vec![child]);
    assert!(matches!(
        cps.address(account).children().await,
        Err(Error::MissingProjectedIdentity)
    ));

    delete(&cps, &alice, child).await;
    delete(&cps, &alice, root).await;
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn reads_through_one_pinned_context_share_a_block() {
    let cps = connect().await;
    let alice = keypair("//Alice");
    let account = alice.account_id();
    let root = create_identified_root(&cps, &alice).await;

    let at = cps.finalized().await.unwrap();
    let block = at.block_hash();

    // Updates after the pin do not show through the pinned context.
    cps.submit_finalized(&cps.node(root).set_payload(b"later").unwrap(), &alice)
        .await
        .unwrap();

    let node = at.node(root);
    assert_eq!(node.payload().await.unwrap().as_deref(), Some(&b"p0"[..]));
    assert_eq!(node.identity().await.unwrap(), Some(account));
    assert_eq!(
        at.address(account).payload().await.unwrap().as_deref(),
        Some(&b"p0"[..])
    );
    assert_eq!(at.block_hash(), block);

    delete(&cps, &alice, root).await;
}
