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
//! Node topology operations.

use crate::{
    api,
    client::Client,
    error::{Error, Result},
    transaction::{self, Transaction},
    types::{CreateNode, NodeId},
};
use std::future::Future;

/// Topology operations: children, child creation and deletion.
///
/// [`children`](NodeTree::children) reads the latest finalized block; the other
/// methods only build a [`Transaction`].
pub trait NodeTree {
    /// Direct children of the node, empty if there are none (or the node does
    /// not exist).
    fn children(&self, client: &Client) -> impl Future<Output = Result<Vec<NodeId>>> + Send;

    /// Build a transaction creating a child node under this node.
    ///
    /// Fails with [`Error::MetaTooLarge`] or [`Error::PayloadTooLarge`] if `params`
    /// exceeds the size limits. The parent must exist and the signer must be
    /// authorized to create nodes under it; the runtime checks both when the
    /// transaction executes.
    fn create_child(&self, params: CreateNode) -> Result<Transaction<NodeId>>;

    /// Build a transaction deleting this node.
    ///
    /// The runtime only allows deleting a node without children, and only by the
    /// owner of the node's scope; deleting a scope root also deletes its scope.
    /// Both are checked when the transaction executes.
    fn delete(&self) -> Transaction<()>;
}

impl NodeTree for NodeId {
    async fn children(&self, client: &Client) -> Result<Vec<NodeId>> {
        let at = client.at_finalized().await?;
        let children = at
            .storage()
            .try_fetch(api::storage().cps().children(), (self.rt(),))
            .await
            .map_err(Error::runtime)?;
        match children {
            Some(value) => Ok(value
                .decode()
                .map_err(Error::runtime)?
                .0
                .into_iter()
                .map(NodeId::from)
                .collect()),
            None => Ok(Vec::new()),
        }
    }

    fn create_child(&self, params: CreateNode) -> Result<Transaction<NodeId>> {
        transaction::create_node(Some(*self), params.meta, params.payload)
    }

    fn delete(&self) -> Transaction<()> {
        transaction::delete_node(*self)
    }
}

/// Build a transaction creating a root node, which starts a new scope owned
/// by the signer.
///
/// Fails with [`Error::MetaTooLarge`] or [`Error::PayloadTooLarge`] if `params`
/// exceeds the size limits.
///
/// Root creation is a free function because no node id exists yet; the
/// created [`NodeId`] is the [`TxReceipt::result`](crate::TxReceipt::result).
pub fn create_root(params: CreateNode) -> Result<Transaction<NodeId>> {
    transaction::create_node(None, params.meta, params.payload)
}
