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

use crate::{
    client::Client,
    error::{Error, Result},
    transaction::{self, Transaction},
    types::{NodeId, ResolvedScope, ScopeId},
};
use parity_scale_codec::{Decode, Encode};
use std::future::Future;

/// SCALE wire format of the `CpsApi::resolve_scope` result.
///
/// The runtime metadata carries no runtime API descriptions, so the API is
/// called by name and its result decoded here.
#[derive(Decode)]
struct ResolvedScopeWire {
    id: crate::api::runtime_types::pallet_robonomics_cps::ScopeId,
    root: crate::api::runtime_types::pallet_robonomics_cps::NodeId,
    owner: crate::types::AccountId,
    path: Vec<crate::api::runtime_types::pallet_robonomics_cps::NodeId>,
}

impl From<ResolvedScopeWire> for ResolvedScope {
    fn from(wire: ResolvedScopeWire) -> Self {
        Self {
            id: wire.id.into(),
            root: wire.root.into(),
            owner: wire.owner,
            path: wire.path.into_iter().map(NodeId::from).collect(),
        }
    }
}

/// Scope resolution and creation, relative to a node.
pub trait NodeScope {
    /// Scope this node belongs to, at the latest finalized block, as resolved by
    /// the runtime (`CpsApi::resolve_scope`).
    ///
    /// Fails with [`Error::NodeNotFound`] if the node does not exist (or no scope
    /// can be resolved for it).
    fn resolve_scope(&self, client: &Client) -> impl Future<Output = Result<ResolvedScope>> + Send;

    /// Build a transaction creating a scope at this node.
    ///
    /// On an ordinary node this creates a new nested scope owned by the
    /// signer. On an existing scope root it replaces the scope with a fresh
    /// [`ScopeId`], which is also how scope ownership changes.
    fn create_scope(&self) -> Transaction<ScopeId>;
}

impl NodeScope for NodeId {
    async fn resolve_scope(&self, client: &Client) -> Result<ResolvedScope> {
        let at = client.at_finalized().await?;
        let resolved: Option<ResolvedScopeWire> =
            Client::cps_api(&at, "resolve_scope", self.rt().encode()).await?;
        resolved.map(Into::into).ok_or(Error::NodeNotFound(*self))
    }

    fn create_scope(&self) -> Transaction<ScopeId> {
        transaction::create_scope(*self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::runtime_types::pallet_robonomics_cps as rt;

    #[test]
    fn resolved_scope_wire_format_decodes() {
        let owner = crate::types::AccountId::from([7u8; 32]);
        let encoded = (
            rt::ScopeId(3),
            rt::NodeId(5),
            owner,
            vec![rt::NodeId(9), rt::NodeId(5)],
        )
            .encode();

        let resolved: ResolvedScope = ResolvedScopeWire::decode(&mut &encoded[..]).unwrap().into();

        assert_eq!(
            resolved,
            ResolvedScope {
                id: ScopeId(3),
                root: NodeId(5),
                owner,
                path: vec![NodeId(9), NodeId(5)],
            }
        );
    }

    #[test]
    fn missing_scope_decodes_to_none() {
        assert!(Option::<ResolvedScopeWire>::decode(&mut &[0u8][..])
            .unwrap()
            .is_none());
    }
}
