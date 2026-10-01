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
//! Node state and data queries.

use crate::{
    api,
    client::Client,
    error::{Error, Result},
    node::watch::{MetaWatcher, PayloadWatcher},
    types::{Meta, NodeId, NodeInfo, Payload},
};
use std::future::Future;

/// Node state and data queries, read at the latest finalized block.
///
/// A missing node is not an error: the queries return `None`. Failures of the
/// query itself are reported as [`Error::Runtime`].
pub trait NodeRead {
    /// Topology of the node, `None` if it does not exist.
    fn info(&self, client: &Client) -> impl Future<Output = Result<Option<NodeInfo>>> + Send;

    /// Metadata of the node, `None` if unset or the node does not exist.
    fn meta(&self, client: &Client) -> impl Future<Output = Result<Option<Meta>>> + Send;

    /// Payload of the node, `None` if unset or the node does not exist.
    fn payload(&self, client: &Client) -> impl Future<Output = Result<Option<Payload>>> + Send;

    /// Subscribe to payload updates of this node in finalized blocks.
    ///
    /// Only blocks finalized after subscribing are observed; see [`PayloadWatcher`].
    fn watch_payload(&self, client: &Client)
        -> impl Future<Output = Result<PayloadWatcher>> + Send;

    /// Subscribe to metadata updates of this node in finalized blocks.
    ///
    /// Only blocks finalized after subscribing are observed; see [`MetaWatcher`].
    fn watch_meta(&self, client: &Client) -> impl Future<Output = Result<MetaWatcher>> + Send;
}

impl NodeRead for NodeId {
    async fn watch_payload(&self, client: &Client) -> Result<PayloadWatcher> {
        PayloadWatcher::new(client, *self).await
    }

    async fn watch_meta(&self, client: &Client) -> Result<MetaWatcher> {
        MetaWatcher::new(client, *self).await
    }

    async fn info(&self, client: &Client) -> Result<Option<NodeInfo>> {
        let at = client.at_finalized().await?;
        let info = at
            .storage()
            .try_fetch(api::storage().cps().nodes(), (self.rt(),))
            .await
            .map_err(Error::runtime)?;
        info.map(|v| v.decode().map_err(Error::runtime))
            .transpose()
            .map(|info| {
                info.map(|info| NodeInfo {
                    parent: info.parent.map(Into::into),
                    scope: info.scope.map(Into::into),
                })
            })
    }

    async fn meta(&self, client: &Client) -> Result<Option<Meta>> {
        let at = client.at_finalized().await?;
        let meta = at
            .storage()
            .try_fetch(api::storage().cps().meta(), (self.rt(),))
            .await
            .map_err(Error::runtime)?;
        meta.map(|v| v.decode().map(|v| v.0).map_err(Error::runtime))
            .transpose()
    }

    async fn payload(&self, client: &Client) -> Result<Option<Payload>> {
        let at = client.at_finalized().await?;
        let payload = at
            .storage()
            .try_fetch(api::storage().cps().payload(), (self.rt(),))
            .await
            .map_err(Error::runtime)?;
        payload
            .map(|v| v.decode().map(|v| v.0).map_err(Error::runtime))
            .transpose()
    }
}
