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
//! Send transactions to a locally running testnet over RPC.
//!
//! Start a development node first (the `robonomics` binary is built from the
//! `airalab/robonomics` repository; it is not part of this workspace):
//!
//! ```text
//! robonomics --dev --tmp
//! ```
//!
//! The development node pre-funds `//Alice`, which this example uses by default.
//! It creates a node tree, updates and reads back its data and finally removes
//! the nodes again.
//!
//! ```text
//! cargo run -p libcps --example rpc_client_write --no-default-features
//! ```
//!
//! Configuration (environment variables):
//! - `ROBONOMICS_WS_URL` - node endpoint, default `ws://127.0.0.1:9944`;
//! - `ROBONOMICS_SURI` - signer secret URI, default `//Alice`.

use libcps::{
    crypto::{Signer, Sr25519},
    prelude::*,
    Client, CreateNode,
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
    let client = Client::connect_rpc(&url).await?;
    println!("Connected to {url} as {}", signer.account_id());

    // Creating a root node also creates a scope owned by the signer.
    let root_tx = libcps::create_root(CreateNode {
        meta: Some(br#"{"type":"building","name":"HQ"}"#.to_vec()),
        payload: Some(br#"{"status":"online"}"#.to_vec()),
    })?;
    let receipt = client.submit_finalized(&root_tx, &signer).await?;
    let root = receipt.result;
    println!("Created root node {root} in block {:?}", receipt.block_hash);

    let child_tx = root.create_child(CreateNode {
        meta: Some(br#"{"type":"room","name":"Server Room"}"#.to_vec()),
        payload: Some(br#"{"temp":"22C"}"#.to_vec()),
    })?;
    let child = client.submit_finalized(&child_tx, &signer).await?.result;
    println!("Created child node {child} under {root}");

    // `submit` returns once the transaction is in the pool, so other work can
    // run before waiting for finality.
    let pending = client
        .submit(&child.set_payload(br#"{"temp":"24C"}"#)?, &signer)
        .await?;
    println!("Submitted extrinsic {:?}", pending.extrinsic_hash());
    let receipt = pending.wait_finalized().await?;
    println!("Payload updated in block {:?}", receipt.block_hash);

    client
        .submit_finalized(
            &child.set_meta(br#"{"type":"room","name":"Lab"}"#)?,
            &signer,
        )
        .await?;

    // Reads see the latest finalized block, which includes the updates above.
    println!("child meta:    {}", text(child.meta(&client).await?));
    println!("child payload: {}", text(child.payload(&client).await?));
    println!("root children: {:?}", root.children(&client).await?);

    // Children must be deleted before their parent.
    client.submit_finalized(&child.delete(), &signer).await?;
    client.submit_finalized(&root.delete(), &signer).await?;
    println!("Deleted nodes {child} and {root}");
    Ok(())
}
