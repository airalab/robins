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
//! Step 1: start the embedded light client and watch it connect to the network.
//!
//! No RPC endpoint and no keys are needed: [`LightNode`] starts a light client for the
//! Polkadot relay chain and the Robonomics parachain (chain specs are embedded in
//! `robonomics-chain-spec`), syncs over p2p and serves a local JSON-RPC endpoint.
//! The light node is stateless, so every run syncs from scratch.
//!
//! The example follows the connection status (`Starting -> RpcReady -> Syncing -> Ready`)
//! and, once the node is ready, connects a [`Cps`] client to the local endpoint and
//! prints the latest finalized block.
//!
//! ```text
//! cargo run -p libcps --example 01_light_client --no-default-features --features light-client
//! ```

// ANCHOR: code
use libcps::{
    light::{LightNode, LightNodeConfig},
    Cps,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let light = LightNode::start(LightNodeConfig::default()).await?;
    println!("Light client serves JSON-RPC at {}", light.rpc_url());

    // `status()` is a watch channel: it always holds the latest state and wakes us
    // on every change, so slow consumers never see a backlog.
    let mut status = light.status();
    loop {
        let current = status.borrow_and_update().clone();
        println!("status: {current}");
        if current.is_ready() {
            break;
        }
        if current.is_terminal() {
            return Err(format!("light client stopped: {current}").into());
        }
        status.changed().await?;
    }

    let cps = Cps::connect(light.rpc_url()).await?;
    let at = cps.finalized().await?;
    println!(
        "Connected to the Robonomics Network, genesis {:?}",
        cps.genesis_hash()
    );
    println!(
        "Latest finalized block #{} ({:?})",
        at.block_number(),
        at.block_hash()
    );

    light.shutdown().await?;
    Ok(())
}
// ANCHOR_END: code
