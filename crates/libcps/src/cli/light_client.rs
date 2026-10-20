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
//! `cps light-client`: run the embedded light client as a local RPC endpoint.

use super::output as display;
use anyhow::Result;
use libcps::light::{LightNode, LightNodeConfig, LightStatus};

/// Default listen address of `cps light-client`.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:9944";

/// Light node configuration for the given command-line options.
pub fn node_config(listen: &str, allow_external: bool) -> LightNodeConfig {
    LightNodeConfig::default()
        .listen(listen)
        .allow_non_loopback(allow_external)
}

/// Run the light client in the foreground until Ctrl-C or failure.
pub async fn execute(config: LightNodeConfig) -> Result<()> {
    display::progress("Starting light client...");
    let node = LightNode::start(config).await?;
    display::success(&format!("Light client RPC listening on {}", node.rpc_url()));
    display::info(&format!(
        "Use it with: cps --ws-url {} <command>",
        node.rpc_url()
    ));

    let mut status = node.status();
    display::info(&format!("Status: {}", *status.borrow_and_update()));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            changed = status.changed() => {
                if changed.is_err() {
                    break;
                }
                let current = status.borrow_and_update().clone();
                display::info(&format!("Status: {current}"));
                if current.is_terminal() {
                    break;
                }
            }
        }
    }

    let failure = match node.current_status() {
        LightStatus::Failed(reason) => Some(reason),
        _ => None,
    };
    display::progress("Shutting down light client...");
    node.shutdown().await?;
    match failure {
        Some(reason) => Err(anyhow::anyhow!("light client failed: {reason}")),
        None => Ok(()),
    }
}
