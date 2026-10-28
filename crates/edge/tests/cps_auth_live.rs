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
//! Live-node integration test of `[auth] mode = "cps"` over real `libcps`.
//!
//! It drives the production path end to end, without any mock reader: the
//! shared [`ChainRegistry`] connection, a finalized snapshot, the
//! identity-to-node lookup and the parent traversal up to the configured root.
//! The same registry serves the policy and a second (anchor-style) handle,
//! which must be one connection.
//!
//! It needs a running Robonomics dev node (runtime v52), so it is `#[ignore]`d
//! by default, like the other live `libcps` tests:
//!
//! ```text
//! LIBCPS_TEST_WS=ws://127.0.0.1:9944 \
//!     cargo test -p edge --test cps_auth_live -- --ignored
//! ```
//!
//! The test deletes the nodes it creates.

use edge::auth::{AuthPolicy, CpsAuth};
use edge::chain::ChainRegistry;
use edge::config::{AuthConfig, AuthMode};
use edge::protocol::SensorIdentity;
use libcps::crypto::{Signer, Sr25519};
use libcps::{Cps, CreateNode, IdentityKey, IdentityScheme, NodeId};
use std::sync::Arc;

fn ws_url() -> String {
    std::env::var("LIBCPS_TEST_WS").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string())
}

/// A node below `parent` whose identity is the sensor derived from `seed`,
/// bound with a permit the sensor signs for the scope of `parent`.
async fn create_sensor_node(
    cps: &Cps,
    owner: &Signer<Sr25519>,
    parent: NodeId,
    seed: u8,
) -> NodeId {
    let key = IdentityKey::from_suri(
        IdentityScheme::Ed25519,
        &format!("0x{}", hex::encode([seed; 32])),
    )
    .expect("sensor seed is a valid key");
    let sensor = SensorIdentity::from_secret_bytes(&[seed; 32]);
    assert_eq!(key.public(), sensor.sensor_id().as_bytes());

    let at = cps.finalized().await.unwrap();
    let scope = at.node(parent).resolve_scope().await.unwrap().id;
    let valid_until = at.block_number() + at.max_identity_permit_ttl().unwrap().min(50);
    let permit = cps
        .identity_permit_payload(scope, valid_until)
        .await
        .unwrap()
        .sign(&key)
        .into_assignment();

    let tx = cps
        .node(parent)
        .create_child(CreateNode::new().identity(permit))
        .unwrap();
    cps.submit_finalized(&tx, owner).await.unwrap().result
}

async fn delete(cps: &Cps, owner: &Signer<Sr25519>, node: NodeId) {
    cps.submit_finalized(&cps.node(node).delete(), owner)
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a live dev node"]
async fn authorizes_descendants_of_the_root_over_a_real_chain() {
    let url = ws_url();
    let cps = Cps::connect(&url).await.expect("dev node is reachable");
    let alice = Signer::<Sr25519>::from_suri("//Alice").unwrap();

    // root ─ branch ─ sensor        (authorized, depth 2)
    // other ─ stranger              (outside the subtree)
    let root = cps
        .submit_finalized(
            &libcps::create_root(CreateNode::new().meta(b"sensors.social")).unwrap(),
            &alice,
        )
        .await
        .unwrap()
        .result;
    let other = cps
        .submit_finalized(
            &libcps::create_root(CreateNode::new().meta(b"other")).unwrap(),
            &alice,
        )
        .await
        .unwrap()
        .result;
    cps.submit_finalized(&cps.node(root).create_scope(), &alice)
        .await
        .ok();
    cps.submit_finalized(&cps.node(other).create_scope(), &alice)
        .await
        .ok();
    let branch = cps
        .submit_finalized(
            &cps.node(root).create_child(CreateNode::new()).unwrap(),
            &alice,
        )
        .await
        .unwrap()
        .result;

    let member = SensorIdentity::from_secret_bytes(&[0x51; 32]);
    let stranger = SensorIdentity::from_secret_bytes(&[0x52; 32]);
    let unbound = SensorIdentity::from_secret_bytes(&[0x53; 32]);
    let member_node = create_sensor_node(&cps, &alice, branch, 0x51).await;
    let stranger_node = create_sensor_node(&cps, &alice, other, 0x52).await;

    let config = AuthConfig {
        mode: AuthMode::Cps,
        cps_root_node_id: Some(root.0),
        cps_expected_genesis_hash: Some(format!("0x{}", hex::encode(cps.genesis_hash().0))),
        cps_negative_ttl_secs: 1,
        ..AuthConfig::default()
    };
    let chains = Arc::new(ChainRegistry::new());
    let policy = CpsAuth::from_config(&config, Some(&url), chains.clone()).unwrap();

    // Authorization never waits for the connection; wait here, as the anchor does.
    let anchor_side = chains.get(Some(&url)).await.expect("connected");
    assert!(
        Arc::ptr_eq(&anchor_side, &chains.try_get(Some(&url)).unwrap()),
        "authorization and anchoring share one connection"
    );

    assert!(policy.authorize(&member.sensor_id()).await.unwrap());
    assert!(!policy.authorize(&stranger.sensor_id()).await.unwrap());
    assert!(!policy.authorize(&unbound.sensor_id()).await.unwrap());

    // The decision is served from the cache now.
    assert_eq!(policy.try_authorize(&member.sensor_id()), Some(true));
    assert_eq!(policy.try_authorize(&stranger.sensor_id()), Some(false));

    delete(&cps, &alice, member_node).await;
    delete(&cps, &alice, stranger_node).await;
    delete(&cps, &alice, branch).await;
    delete(&cps, &alice, root).await;
    delete(&cps, &alice, other).await;
}
