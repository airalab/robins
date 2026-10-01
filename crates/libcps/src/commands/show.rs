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
//! Show node tree command implementation.

use super::Connection;
use crate::display;
use anyhow::Result;
use libcps::crypto::{Cipher, EncryptedMessage, Scheme, Signer};
use libcps::{prelude::*, Client, NodeId};
use parity_scale_codec::Decode;
use std::future::Future;
use std::pin::Pin;

/// Print `node_id` and its descendants as a tree, decrypting data with `cipher`
/// when one is given.
pub async fn execute<S: Scheme>(
    connection: &Connection,
    cipher: Option<&Signer<S>>,
    node_id: u64,
) -> Result<()> {
    let client = connection.client().await?;

    display::progress(&format!("Fetching node tree from node {node_id}..."));

    print_node_tree(&client, NodeId::from(node_id), cipher, "", true).await?;

    Ok(())
}

/// Render stored bytes as text: encrypted messages are decrypted with `cipher`, or
/// shown as JSON when there is no cipher; anything else must be UTF-8.
fn data_to_string<S: Scheme>(data: Vec<u8>, cipher: Option<&Signer<S>>) -> Result<String> {
    if let Ok(message) = EncryptedMessage::decode(&mut data.as_slice()) {
        if let Some(cipher) = cipher {
            let decrypted = cipher
                .decrypt(&message, None)
                .map_err(|e| anyhow::anyhow!("Failed to decrypt message: {}.", e))?;
            String::from_utf8(decrypted).map_err(|_| anyhow::anyhow!("Invalid UTF-8 character"))
        } else {
            serde_json::to_string(&message)
                .map_err(|e| {
                    anyhow::anyhow!("Failed to convert encrypted message into JSON: {}.", e)
                })
                .map(|json_msg| format!("Encrypted: {}", json_msg))
        }
    } else {
        String::from_utf8(data).map_err(|_| anyhow::anyhow!("Invalid UTF-8 character"))
    }
}

fn print_node_tree<'a, S: Scheme>(
    client: &'a Client,
    node: NodeId,
    cipher: Option<&'a Signer<S>>,
    prefix: &'a str,
    is_last: bool,
) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
    Box::pin(async move {
        if node.info(client).await?.is_none() {
            return Err(libcps::Error::NodeNotFound(node).into());
        }

        let owner = node.resolve_scope(client).await?.owner;
        let meta = node
            .meta(client)
            .await?
            .map(|meta| data_to_string(meta, cipher))
            .transpose()?;
        let payload = node
            .payload(client)
            .await?
            .map(|payload| data_to_string(payload, cipher))
            .transpose()?;
        let children = node.children(client).await?;

        display::tree::print_node_recursive(
            node.0,
            owner,
            meta.as_deref(),
            payload.as_deref(),
            prefix,
            is_last,
        );

        if !children.is_empty() {
            let child_prefix = if is_last {
                format!("{}    ", prefix)
            } else {
                format!("{}|   ", prefix)
            };

            for (i, child) in children.iter().enumerate() {
                let is_last_child = i == children.len() - 1;
                print_node_tree(client, *child, cipher, &child_prefix, is_last_child).await?;
            }
        }

        Ok(())
    })
}
