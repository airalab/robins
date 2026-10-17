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
//! Follow node data changes with watchers on a local testnet.
//!
//! [`Node::watch_payload`] and [`Node::watch_meta`] yield an update for every finalized
//! block that changes the field, with the block hash and number. The example runs a
//! writer and a watcher side by side and stops after the third update.
//!
//! Start a development node first (see `rpc_client_write`), then:
//!
//! ```text
//! cargo run -p libcps --example watch_payload --no-default-features
//! ```
//!
//! Environment: `ROBONOMICS_WS_URL` (default `ws://127.0.0.1:9944`) and
//! `ROBONOMICS_SURI` (default `//Alice`).

use libcps::{
    crypto::{Signer, Sr25519},
    Cps, CreateNode, NodeId,
};

/// Print the first `count` payload updates of `node`.
async fn watch(cps: &Cps, node: NodeId, count: usize) -> libcps::Result<()> {
    let mut watcher = cps.node(node).watch_payload().await?;
    for _ in 0..count {
        match watcher.next().await {
            Some(Ok(update)) => println!(
                "block {} ({:?}): payload {:?}",
                update.block_number,
                update.block_hash,
                update
                    .payload
                    .map(|p| String::from_utf8_lossy(&p).into_owned())
            ),
            Some(Err(e)) => return Err(e),
            None => break,
        }
    }
    Ok(())
}

/// Write three payloads, then clear it, one finalized transaction each.
async fn write(cps: &Cps, signer: &Signer<Sr25519>, node: NodeId) -> libcps::Result<()> {
    for i in 1..=3 {
        let tx = cps.node(node).set_payload(format!("reading #{i}"))?;
        cps.submit_finalized(&tx, signer).await?;
    }
    cps.submit_finalized(&cps.node(node).clear_payload(), signer)
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url =
        std::env::var("ROBONOMICS_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:9944".to_string());
    let suri = std::env::var("ROBONOMICS_SURI").unwrap_or_else(|_| "//Alice".to_string());
    let signer = Signer::<Sr25519>::from_suri(&suri)?;
    let cps = Cps::connect_rpc(&url).await?;

    let node = cps
        .submit_finalized(&libcps::create_root(CreateNode::new())?, &signer)
        .await?
        .result;
    println!("watching node {node}");

    // The watcher starts from the next block on, so start it before writing.
    let (watched, written) = tokio::join!(watch(&cps, node, 3), write(&cps, &signer, node));
    watched?;
    written?;

    cps.submit_finalized(&cps.node(node).delete(), &signer)
        .await?;
    Ok(())
}
