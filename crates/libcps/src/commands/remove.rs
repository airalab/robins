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
//! Remove node command implementation.

use super::Connection;
use crate::display;
use anyhow::Result;
use colored::*;
use libcps::{prelude::*, NodeId};
use std::io::{self, Write};

/// Delete `node_id`; refuses when it has children, and asks for confirmation
/// unless `force` is set.
pub async fn execute(connection: &Connection, node_id: u64, force: bool) -> Result<()> {
    let client = connection.client().await?;
    let signer = connection.signer()?;

    let node = NodeId::from(node_id);
    if node.info(&client).await?.is_none() {
        return Err(libcps::Error::NodeNotFound(node).into());
    }

    let children = node.children(&client).await?;
    if !children.is_empty() {
        return Err(anyhow::anyhow!(
            "Cannot delete node with {} children. Remove the children first",
            children.len()
        ));
    }

    if !force {
        print!(
            "{} Are you sure you want to delete node {}? (y/N): ",
            "[!]".yellow().bold(),
            node_id.to_string().bright_cyan()
        );
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            display::info("Deletion cancelled");
            return Ok(());
        }
    }

    let tx = node.delete();

    let spinner = display::spinner("Submitting transaction...");
    let receipt = client.submit_finalized(&tx, &signer).await;
    spinner.finish_and_clear();
    receipt?;

    display::success(&format!(
        "Node {} deleted",
        node_id.to_string().bright_cyan()
    ));

    Ok(())
}
