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
//! Step 5: create your own CPS tree on a local development node.
//!
//! Start a development chain first. There is no standalone node binary: in a checkout
//! of `airalab/robonomics`, build and run the development chain as a Docker image
//! (see "Running a Development Chain" in its `DEVELOPER_GUIDE.md`):
//!
//! ```text
//! nix develop -c ./scripts/development-chain.sh docker
//! docker tag "$(docker images -q | head -n 1)" robonomics-dev
//! docker run --rm -p 9944:9944 robonomics-dev
//! ```
//!
//! The development node pre-funds `//Alice`, which this example uses by default.
//! It creates a node tree, updates and reads back its data and finally removes
//! the nodes again.
//!
//! ```text
//! cargo run -p libcps --example 05_local_node_tree --no-default-features
//! ```
//!
//! Configuration (environment variables):
//! - `ROBONOMICS_WS_URL` - node endpoint, default `ws://127.0.0.1:9944`;
//! - `ROBONOMICS_SURI` - signer secret URI, default `//Alice`.

// ANCHOR: code
use libcps::{
    crypto::{Signer, Sr25519},
    Cps, CreateNode, IdentityAssignment,
};

/// Text of a stored value, for printing.
fn text(data: Option<Vec<u8>>) -> String {
    data.map(|data| String::from_utf8_lossy(&data).into_owned())
        .unwrap_or_else(|| "<none>".to_string())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url =
        std::env::var("ROBONOMICS_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string());
    let suri = std::env::var("ROBONOMICS_SURI").unwrap_or_else(|_| "//Alice".to_string());

    let signer = Signer::<Sr25519>::from_suri(&suri)?;
    let cps = Cps::connect(&url).await?;
    println!("Connected to {url} as {}", signer.account_id());

    // Creating a root node also creates a scope owned by the signer. A root can
    // only get the signer itself as identity, which makes it addressable by account.
    let root_tx = libcps::create_root(
        CreateNode::new()
            .meta(br#"{"type":"building","name":"HQ"}"#)
            .payload(br#"{"status":"online"}"#)
            .identity(IdentityAssignment::new(signer.account_id())),
    )?;
    let receipt = cps.submit_finalized(&root_tx, &signer).await?;
    let root = receipt.result;
    println!("Created root node {root} in block {:?}", receipt.block_hash);

    let child_tx = cps.node(root).create_child(
        CreateNode::new()
            .meta(br#"{"type":"room","name":"Server Room"}"#)
            .payload(br#"{"temp":"22C"}"#),
    )?;
    let child = cps.submit_finalized(&child_tx, &signer).await?.result;
    println!("Created child node {child} under {root}");

    // `submit` returns once the transaction is in the pool, so other work can
    // run before waiting for finality.
    let child_node = cps.node(child);
    let pending = cps
        .submit(&child_node.set_payload(br#"{"temp":"24C"}"#)?, &signer)
        .await?;
    println!("Submitted extrinsic {:?}", pending.extrinsic_hash());
    let receipt = pending.wait_finalized().await?;
    println!("Payload updated in block {:?}", receipt.block_hash);

    cps.submit_finalized(
        &child_node.set_meta(br#"{"type":"room","name":"Lab"}"#)?,
        &signer,
    )
    .await?;

    // Reads see the latest finalized block, which includes the updates above.
    // `CpsAt` pins all reads below to that one block.
    let at = cps.finalized().await?;
    println!("Reading at block {:?}", at.block_hash());
    let child_state = at.node(child);
    println!("child meta:    {}", text(child_state.meta().await?));
    println!("child payload: {}", text(child_state.payload().await?));
    println!("root children: {:?}", at.node(root).children().await?);

    // The same root, addressed by its identity account instead of its node ID.
    let by_account = at.address(signer.account_id());
    println!(
        "identity {} is node {:?}",
        signer.account_id(),
        by_account.node_id().await?
    );
    println!(
        "root payload by account: {}",
        text(by_account.payload().await?)
    );

    // Children must be deleted before their parent.
    cps.submit_finalized(&child_node.delete(), &signer).await?;
    cps.submit_finalized(&cps.node(root).delete(), &signer)
        .await?;
    println!("Deleted nodes {child} and {root}");
    Ok(())
}
// ANCHOR_END: code
