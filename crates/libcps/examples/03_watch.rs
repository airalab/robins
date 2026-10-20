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
//! Step 3: watch a live node on the Robonomics Network.
//!
//! [`Node::watch_payload`] and [`Node::watch_meta`] yield an update for every finalized
//! block that changes the field, with the block hash and number. Pointed at a node a
//! sensor keeps writing to (such as a sensor.social node), the example prints every new
//! reading as it is finalized. Nothing is written: the example only reads, over the
//! embedded light client, so no keys or funds are needed.
//!
//! The node is taken from the first argument, `CPS_WATCH_NODE` or `CPS_ROOT_NODE`, in
//! this order. The example stops after `CPS_WATCH_COUNT` updates (default 5).
//!
//! ```text
//! cargo run -p libcps --example 03_watch --no-default-features \
//!     --features light-client -- <NODE_ID>
//! ```

// ANCHOR: code
use libcps::{
    light::{LightNode, LightNodeConfig},
    Cps, NodeId,
};

/// Render a stored value for printing, hex for anything that is not UTF-8.
fn render(data: Option<Vec<u8>>) -> String {
    match data {
        None => "<cleared>".to_string(),
        Some(data) => match String::from_utf8(data) {
            Ok(text) => text,
            Err(e) => format!("0x{}", hex::encode(e.into_bytes())),
        },
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let node: u64 = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CPS_WATCH_NODE").ok())
        .or_else(|| std::env::var("CPS_ROOT_NODE").ok())
        .ok_or("usage: 03_watch <NODE_ID> (or set CPS_WATCH_NODE)")?
        .parse()
        .map_err(|e| format!("invalid node id: {e}"))?;
    let count: usize = std::env::var("CPS_WATCH_COUNT")
        .ok()
        .map(|count| count.parse())
        .transpose()
        .map_err(|e| format!("invalid CPS_WATCH_COUNT: {e}"))?
        .unwrap_or(5);

    println!("Starting light client, waiting for the first finalized block...");
    let light = LightNode::start(LightNodeConfig::default()).await?;
    light.wait_ready().await?;
    let cps = Cps::connect(light.rpc_url()).await?;

    println!("Watching node {node}, waiting for {count} updates (Ctrl+C to stop)");
    let mut watcher = cps.node(NodeId::from(node)).watch_payload().await?;
    for _ in 0..count {
        match watcher.next().await {
            Some(Ok(update)) => println!(
                "block #{} ({:?}): {}",
                update.block_number,
                update.block_hash,
                render(update.payload)
            ),
            Some(Err(e)) => return Err(e.into()),
            None => break,
        }
    }

    light.shutdown().await?;
    Ok(())
}
// ANCHOR_END: code
