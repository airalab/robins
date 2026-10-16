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
//! Node-relative scope operations.

use crate::api::runtime_types::pallet_robonomics_cps as rt;
use crate::{
    api,
    backend::At,
    client::Client,
    error::{Error, Result},
    transaction::{self, Transaction},
    types::{Hash, NodeId, ResolvedScope, ScopeId},
};
use std::future::Future;

impl From<rt::ResolvedScope<crate::types::AccountId>> for ResolvedScope {
    fn from(scope: rt::ResolvedScope<crate::types::AccountId>) -> Self {
        Self {
            id: scope.id.into(),
            root: scope.root.into(),
            owner: scope.owner,
            path: scope.path.0.into_iter().map(NodeId::from).collect(),
        }
    }
}

/// Scope resolution and creation, relative to a node.
pub trait NodeScope {
    /// Scope this node belongs to, at the latest finalized block, as resolved by
    /// the runtime (the CPS `resolve_scope` view function).
    ///
    /// Fails with [`Error::NodeNotFound`] if the node does not exist (or no scope
    /// can be resolved for it).
    fn resolve_scope(&self, client: &Client) -> impl Future<Output = Result<ResolvedScope>> + Send;

    /// As [`resolve_scope`](NodeScope::resolve_scope), at the block with hash
    /// `block_hash`.
    fn resolve_scope_at(
        &self,
        client: &Client,
        block_hash: Hash,
    ) -> impl Future<Output = Result<ResolvedScope>> + Send;

    /// Build a transaction creating a scope at this node.
    ///
    /// On an ordinary node this creates a new nested scope owned by the
    /// signer. On an existing scope root it replaces the scope with a fresh
    /// [`ScopeId`], which is also how scope ownership changes.
    fn create_scope(&self) -> Transaction<ScopeId>;
}

impl NodeScope for NodeId {
    async fn resolve_scope(&self, client: &Client) -> Result<ResolvedScope> {
        resolve(*self, &client.at_finalized().await?).await
    }

    async fn resolve_scope_at(&self, client: &Client, block_hash: Hash) -> Result<ResolvedScope> {
        resolve(*self, &client.at_block(block_hash).await?).await
    }

    fn create_scope(&self) -> Transaction<ScopeId> {
        transaction::create_scope(*self)
    }
}

/// Scope of `node` resolved by the runtime at the block `at`.
async fn resolve(node: NodeId, at: &At) -> Result<ResolvedScope> {
    let resolved = at
        .view_functions()
        .call(api::view_functions().cps().resolve_scope(node.rt()))
        .await
        .map_err(Error::runtime)?;
    super::view_result(node, resolved).map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::runtime_types::{
        bounded_collections::bounded_vec::BoundedVec, pallet_robonomics_cps as rt,
    };

    #[test]
    fn runtime_resolved_scope_converts() {
        let owner = crate::types::AccountId::from([7u8; 32]);
        let runtime = rt::ResolvedScope {
            id: rt::ScopeId(3),
            root: rt::NodeId(5),
            owner,
            path: BoundedVec(vec![rt::NodeId(9), rt::NodeId(5)]),
        };

        assert_eq!(
            ResolvedScope::from(runtime),
            ResolvedScope {
                id: ScopeId(3),
                root: NodeId(5),
                owner,
                path: vec![NodeId(9), NodeId(5)],
            }
        );
    }

    #[test]
    fn missing_node_maps_to_node_not_found() {
        use rt::pallet::Error as PalletError;

        let err =
            super::super::view_result::<()>(NodeId(4), Err(PalletError::NodeNotFound)).unwrap_err();
        assert!(matches!(err, Error::NodeNotFound(NodeId(4))));

        let err = super::super::view_result::<()>(NodeId(4), Err(PalletError::ScopeNotFound))
            .unwrap_err();
        assert!(matches!(err, Error::Runtime(_)));
    }
}
