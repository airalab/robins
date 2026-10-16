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
//! Subscriptions to metadata and payload updates of a node.

use crate::{
    api,
    client::Client,
    error::{Error, Result},
    types::{Hash, Meta, NodeId, Payload},
};
use robonomics_runtime_subxt_api::RobonomicsConfig;

/// Node data a watcher follows.
#[derive(Clone, Copy)]
enum Field {
    Meta,
    Payload,
}

/// A value change of a watched field, observed in a finalized block.
struct Change {
    block_hash: Hash,
    block_number: u64,
    value: Option<Vec<u8>>,
}

/// Finalized-block stream shared by [`MetaWatcher`] and [`PayloadWatcher`].
struct BlockWatcher {
    node: NodeId,
    field: Field,
    blocks: subxt::client::Blocks<RobonomicsConfig>,
}

impl BlockWatcher {
    async fn new(client: &Client, node: NodeId, field: Field) -> Result<Self> {
        let blocks = client
            .api()
            .stream_blocks()
            .await
            .map_err(|e| Error::Rpc(Box::new(e)))?;
        Ok(Self {
            node,
            field,
            blocks,
        })
    }

    /// Next finalized block that sets the watched field of the node.
    ///
    /// `None` when the subscription ends. An error concerns a single block.
    async fn next(&mut self) -> Option<Result<Change>> {
        loop {
            let block = match self.blocks.next().await? {
                Ok(block) => block,
                Err(e) => return Some(Err(Error::Rpc(Box::new(e)))),
            };
            match Self::change_in(self.node, self.field, &block).await {
                Ok(Some(change)) => return Some(Ok(change)),
                Ok(None) => continue,
                Err(e) => return Some(Err(e)),
            }
        }
    }

    async fn change_in(
        node: NodeId,
        field: Field,
        block: &subxt::client::Block<RobonomicsConfig>,
    ) -> Result<Option<Change>> {
        let at = block.at().await.map_err(Error::runtime)?;
        let events = at.events().fetch().await.map_err(Error::runtime)?;
        let updated = match field {
            Field::Meta => events
                .find::<api::cps::events::MetaSet>()
                .filter_map(|event| event.ok())
                .any(|event| NodeId::from(event.0) == node),
            Field::Payload => events
                .find::<api::cps::events::PayloadSet>()
                .filter_map(|event| event.ok())
                .any(|event| NodeId::from(event.0) == node),
        };
        if !updated {
            return Ok(None);
        }

        let storage = at.storage();
        let value = match field {
            Field::Meta => storage
                .try_fetch(api::storage().cps().meta(), (node.rt(),))
                .await
                .map_err(Error::runtime)?
                .map(|value| value.decode().map(|v| v.0).map_err(Error::runtime))
                .transpose()?,
            Field::Payload => storage
                .try_fetch(api::storage().cps().payload(), (node.rt(),))
                .await
                .map_err(Error::runtime)?
                .map(|value| value.decode().map(|v| v.0).map_err(Error::runtime))
                .transpose()?,
        };
        Ok(Some(Change {
            block_hash: block.hash(),
            block_number: block.number(),
            value,
        }))
    }
}

/// A payload update of a watched node, observed in a finalized block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PayloadUpdate {
    /// Finalized block that set the payload.
    pub block_hash: Hash,
    /// Number of that block.
    pub block_number: u64,
    /// Payload after the update, `None` if it was cleared.
    pub payload: Option<Payload>,
}

/// Stream of payload updates of one node, from finalized blocks.
///
/// Created by [`NodeRead::watch_payload`](crate::NodeRead::watch_payload).
pub struct PayloadWatcher(BlockWatcher);

impl PayloadWatcher {
    pub(crate) async fn new(client: &Client, node: NodeId) -> Result<Self> {
        BlockWatcher::new(client, node, Field::Payload)
            .await
            .map(Self)
    }

    /// Wait for the next finalized block that sets (or clears) the node payload.
    ///
    /// Only blocks finalized after the subscription started are observed. Returns
    /// `None` when the subscription ends. A returned error concerns a single block
    /// ([`Error::Rpc`] for the block stream, [`Error::Runtime`] for reading it); the
    /// watcher can be polled again afterwards.
    pub async fn next(&mut self) -> Option<Result<PayloadUpdate>> {
        let change = self.0.next().await?;
        Some(change.map(|change| PayloadUpdate {
            block_hash: change.block_hash,
            block_number: change.block_number,
            payload: change.value,
        }))
    }
}

/// A metadata update of a watched node, observed in a finalized block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetaUpdate {
    /// Finalized block that set the metadata.
    pub block_hash: Hash,
    /// Number of that block.
    pub block_number: u64,
    /// Metadata after the update, `None` if it was cleared.
    pub meta: Option<Meta>,
}

/// Stream of metadata updates of one node, from finalized blocks.
///
/// Created by [`NodeRead::watch_meta`](crate::NodeRead::watch_meta).
pub struct MetaWatcher(BlockWatcher);

impl MetaWatcher {
    pub(crate) async fn new(client: &Client, node: NodeId) -> Result<Self> {
        BlockWatcher::new(client, node, Field::Meta).await.map(Self)
    }

    /// Wait for the next finalized block that sets (or clears) the node metadata.
    ///
    /// Only blocks finalized after the subscription started are observed. Returns
    /// `None` when the subscription ends. A returned error concerns a single block
    /// ([`Error::Rpc`] for the block stream, [`Error::Runtime`] for reading it); the
    /// watcher can be polled again afterwards.
    pub async fn next(&mut self) -> Option<Result<MetaUpdate>> {
        let change = self.0.next().await?;
        Some(change.map(|change| MetaUpdate {
            block_hash: change.block_hash,
            block_number: change.block_number,
            meta: change.value,
        }))
    }
}
