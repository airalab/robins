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
//! Read the production chain with the embedded light client.
//!
//! No RPC endpoint and no keys are needed: [`LightNode`] starts a light client
//! for the Polkadot relay chain and the Robonomics parachain (chain specs are
//! embedded in `robonomics-chain-spec`), syncs over p2p and serves a local
//! JSON-RPC endpoint that the RPC-only [`Cps`] connects to. The light node is
//! stateless, so every run syncs from scratch and may take a while until a
//! finalized block is available.
//!
//! The example prints the whole CPS tree below a root node. All reads are pinned
//! to one finalized block, so the printed tree is a consistent snapshot.
//!
//! ```text
//! cargo run -p libcps --example light_client_read --no-default-features \
//!     --features light-client -- <ROOT_NODE_ID>
//! # or
//! CPS_ROOT_NODE=<ROOT_NODE_ID> cargo run -p libcps --example light_client_read --no-default-features \
//!     --features light-client
//! ```

use libcps::{
    light::{LightNode, LightNodeConfig},
    Cps, CpsAt, Error, NodeId,
};
use std::{future::Future, pin::Pin};

/// Longest value, in bytes, printed in full; longer values are cut.
const PREVIEW_LEN: usize = 64;

/// Render stored bytes: UTF-8 text as is, anything else (e.g. encrypted data) as hex.
fn render(data: &[u8]) -> String {
    let shown = &data[..data.len().min(PREVIEW_LEN)];
    let mut text = match std::str::from_utf8(shown) {
        Ok(text) => format!("{text:?}"),
        Err(_) => format!("0x{}", hex::encode(shown)),
    };
    if data.len() > PREVIEW_LEN {
        text.push_str(&format!("... ({} bytes)", data.len()));
    }
    text
}

/// Print `node` and its descendants, reading the state of the pinned block `at`.
///
/// Recursion in `async fn` needs a boxed future.
fn print_tree<'a>(
    at: &'a CpsAt,
    node: NodeId,
    depth: usize,
) -> Pin<Box<dyn Future<Output = libcps::Result<()>> + 'a>> {
    Box::pin(async move {
        let handle = at.node(node);
        if handle.info().await?.is_none() {
            return Err(Error::NodeNotFound(node));
        }

        let indent = "  ".repeat(depth);
        let scope = handle.resolve_scope().await?;
        println!("{indent}[node {node}] owner: {}", scope.owner);
        if let Some(meta) = handle.meta().await? {
            println!("{indent}  meta:    {}", render(&meta));
        }
        if let Some(payload) = handle.payload().await? {
            println!("{indent}  payload: {}", render(&payload));
        }

        for child in handle.children().await? {
            print_tree(at, child, depth + 1).await?;
        }
        Ok(())
    })
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root: u64 = std::env::args()
        .nth(1)
        .or_else(|| std::env::var("CPS_ROOT_NODE").ok())
        .ok_or("usage: light_client_read <ROOT_NODE_ID> (or set CPS_ROOT_NODE)")?
        .parse()
        .map_err(|e| format!("invalid root node id: {e}"))?;

    println!("Starting light client, waiting for the first finalized block...");
    let light = LightNode::start(LightNodeConfig::default()).await?;
    light.wait_ready().await?;
    let cps = Cps::connect(light.rpc_url()).await?;

    let at = cps.finalized().await?;
    println!("Synced, reading at finalized block {:?}\n", at.block_hash());

    print_tree(&at, NodeId::from(root), 0).await?;
    light.shutdown().await?;
    Ok(())
}
