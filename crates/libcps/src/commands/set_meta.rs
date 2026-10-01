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
//! Set metadata command implementation.

use super::{encrypt, Connection};
use crate::display;
use anyhow::Result;
use colored::*;
use libcps::crypto::{EncryptionAlgorithm, Scheme, Signer};
use libcps::{prelude::*, NodeId};

/// Replace the metadata of `node_id`, encrypting it with `cipher` for
/// `receiver_public` when a receiver is given.
pub async fn execute<S: Scheme>(
    connection: &Connection,
    cipher: Option<&Signer<S>>,
    node_id: u64,
    data: String,
    receiver_public: Option<[u8; 32]>,
    algorithm: Option<EncryptionAlgorithm>,
) -> Result<()> {
    let client = connection.client().await?;
    let signer = connection.signer()?;

    display::info(&format!("Updating metadata for node {node_id}"));

    let bytes = match receiver_public.as_ref() {
        Some(receiver) => encrypt("metadata", cipher, algorithm, receiver, data.as_bytes())?,
        None => data.into_bytes(),
    };

    let tx = NodeId::from(node_id).set_meta(bytes)?;

    let spinner = display::spinner("Submitting transaction...");
    let receipt = client.submit_finalized(&tx, &signer).await;
    spinner.finish_and_clear();
    receipt?;

    display::success(&format!(
        "Metadata updated for node {}",
        node_id.to_string().bright_cyan()
    ));

    Ok(())
}
