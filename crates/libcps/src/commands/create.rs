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
//! Create node command implementation.

use super::{encrypt, Connection};
use crate::display;
use anyhow::Result;
use colored::*;
use libcps::crypto::{EncryptionAlgorithm, Scheme, Signer};
use libcps::{prelude::*, CreateNode, NodeId};

/// Create a root node (`parent == None`, which starts a new scope owned by the
/// signer) or a child node.
///
/// Metadata and payload are encrypted with `cipher` for `receiver_public` when a
/// receiver is given.
pub async fn execute<S: Scheme>(
    connection: &Connection,
    cipher: Option<&Signer<S>>,
    parent: Option<u64>,
    meta: Option<String>,
    payload: Option<String>,
    receiver_public: Option<[u8; 32]>,
    algorithm: Option<EncryptionAlgorithm>,
) -> Result<()> {
    let client = connection.client().await?;
    let signer = connection.signer()?;

    match parent {
        Some(parent) => display::info(&format!("Creating child node under parent {parent}")),
        None => display::info("Creating root node"),
    }

    let seal = |what: &str, data: Option<String>| -> Result<Option<Vec<u8>>> {
        match (data, receiver_public.as_ref()) {
            (Some(data), Some(receiver)) => {
                encrypt(what, cipher, algorithm, receiver, data.as_bytes()).map(Some)
            }
            (data, _) => Ok(data.map(String::into_bytes)),
        }
    };
    let params = CreateNode {
        meta: seal("metadata", meta)?,
        payload: seal("payload", payload)?,
    };

    let tx = match parent {
        Some(parent) => NodeId::from(parent).create_child(params)?,
        None => libcps::create_root(params)?,
    };

    let spinner = display::spinner("Submitting transaction...");
    let receipt = client.submit_finalized(&tx, &signer).await;
    spinner.finish_and_clear();
    let receipt = receipt?;

    display::success(&format!(
        "Node created with ID: {}",
        receipt.result.to_string().bright_cyan()
    ));

    Ok(())
}
