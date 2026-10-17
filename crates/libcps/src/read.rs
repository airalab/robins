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
//! Crate-private chain reads at a fixed block.
//!
//! Every function reads from the given [`At`], so that all reads sharing one
//! [`At`] observe the same block. CPS semantics (scope resolution, capability
//! checks, address projection) are delegated to the runtime view functions.

use crate::{
    api::{self, runtime_types::pallet_robonomics_cps as rt},
    backend::At,
    error::{Error, Result},
    types::{
        AccountId, Capability, Meta, NodeId, NodeInfo, Payload, ResolvedScope, ScopeId, ScopeInfo,
    },
};

/// Map a node view function failure to [`Error`].
fn node_view<T>(node: NodeId, result: Result<T, rt::pallet::Error>) -> Result<T> {
    result.map_err(|e| match e {
        rt::pallet::Error::NodeNotFound => Error::NodeNotFound(node),
        other => Error::runtime(format!("CPS view function failed: {other:?}")),
    })
}

/// Map an address view function failure to [`Error`].
fn address_view<T>(account: AccountId, result: Result<T, rt::AddressViewError>) -> Result<T> {
    result.map_err(|e| match e {
        rt::AddressViewError::IdentityNotFound => Error::IdentityNotFound(account),
        rt::AddressViewError::MissingProjectedIdentity => Error::MissingProjectedIdentity,
    })
}

impl From<rt::ResolvedScope<AccountId>> for ResolvedScope {
    fn from(scope: rt::ResolvedScope<AccountId>) -> Self {
        Self {
            id: scope.id.into(),
            root: scope.root.into(),
            owner: scope.owner,
            path: scope.path.0.into_iter().map(NodeId::from).collect(),
        }
    }
}

pub(crate) async fn info(at: &At, node: NodeId) -> Result<Option<NodeInfo>> {
    let info = at
        .storage()
        .try_fetch(api::storage().cps().nodes(), (node.rt(),))
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

pub(crate) async fn meta(at: &At, node: NodeId) -> Result<Option<Meta>> {
    let meta = at
        .storage()
        .try_fetch(api::storage().cps().meta(), (node.rt(),))
        .await
        .map_err(Error::runtime)?;
    meta.map(|v| v.decode().map(|v| v.0).map_err(Error::runtime))
        .transpose()
}

pub(crate) async fn payload(at: &At, node: NodeId) -> Result<Option<Payload>> {
    let payload = at
        .storage()
        .try_fetch(api::storage().cps().payload(), (node.rt(),))
        .await
        .map_err(Error::runtime)?;
    payload
        .map(|v| v.decode().map(|v| v.0).map_err(Error::runtime))
        .transpose()
}

pub(crate) async fn children(at: &At, node: NodeId) -> Result<Vec<NodeId>> {
    let children = at
        .storage()
        .try_fetch(api::storage().cps().children(), (node.rt(),))
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

pub(crate) async fn resolve_scope(at: &At, node: NodeId) -> Result<ResolvedScope> {
    let resolved = at
        .view_functions()
        .call(api::view_functions().cps().resolve_scope(node.rt()))
        .await
        .map_err(Error::runtime)?;
    node_view(node, resolved).map(Into::into)
}

pub(crate) async fn has_capability(
    at: &At,
    node: NodeId,
    account: &AccountId,
    capability: Capability,
) -> Result<bool> {
    let result = at
        .view_functions()
        .call(
            api::view_functions()
                .cps()
                .has_capability(node.rt(), *account, capability.into()),
        )
        .await
        .map_err(Error::runtime)?;
    node_view(node, result)
}

pub(crate) async fn identity_of(at: &At, node: NodeId) -> Result<Option<AccountId>> {
    at.view_functions()
        .call(api::view_functions().cps().identity_of(node.rt()))
        .await
        .map_err(Error::runtime)
}

pub(crate) async fn node_by_identity(at: &At, account: AccountId) -> Result<Option<NodeId>> {
    let node = at
        .view_functions()
        .call(api::view_functions().cps().node_by_identity(account))
        .await
        .map_err(Error::runtime)?;
    Ok(node.map(Into::into))
}

pub(crate) async fn address_parent(at: &At, account: AccountId) -> Result<Option<AccountId>> {
    let result = at
        .view_functions()
        .call(api::view_functions().cps().parent(account))
        .await
        .map_err(Error::runtime)?;
    address_view(account, result)
}

pub(crate) async fn address_children(at: &At, account: AccountId) -> Result<Vec<AccountId>> {
    let result = at
        .view_functions()
        .call(api::view_functions().cps().children(account))
        .await
        .map_err(Error::runtime)?;
    address_view(account, result)
}

pub(crate) async fn address_meta(at: &At, account: AccountId) -> Result<Option<Meta>> {
    let result = at
        .view_functions()
        .call(api::view_functions().cps().meta(account))
        .await
        .map_err(Error::runtime)?;
    address_view(account, result).map(|meta| meta.map(|meta| meta.0))
}

pub(crate) async fn address_payload(at: &At, account: AccountId) -> Result<Option<Payload>> {
    let result = at
        .view_functions()
        .call(api::view_functions().cps().payload(account))
        .await
        .map_err(Error::runtime)?;
    address_view(account, result).map(|payload| payload.map(|payload| payload.0))
}

pub(crate) async fn address_scope(at: &At, account: AccountId) -> Result<ScopeId> {
    let result = at
        .view_functions()
        .call(api::view_functions().cps().scope_of(account))
        .await
        .map_err(Error::runtime)?;
    address_view(account, result).map(Into::into)
}

pub(crate) async fn address_capability(
    at: &At,
    caller: AccountId,
    target: AccountId,
    capability: Capability,
) -> Result<bool> {
    let result = at
        .view_functions()
        .call(
            api::view_functions()
                .cps()
                .address_capability(caller, target, capability.into()),
        )
        .await
        .map_err(Error::runtime)?;
    address_view(target, result)
}

pub(crate) async fn scope_info(at: &At, scope: ScopeId) -> Result<Option<ScopeInfo>> {
    let info = at
        .storage()
        .try_fetch(api::storage().cps().scopes(), (scope.rt(),))
        .await
        .map_err(Error::runtime)?;
    info.map(|v| v.decode().map_err(Error::runtime))
        .transpose()
        .map(|info| {
            info.map(|info| ScopeInfo {
                owner: info.owner,
                access_count: info.access_count,
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::runtime_types::bounded_collections::bounded_vec::BoundedVec;

    #[test]
    fn runtime_resolved_scope_converts() {
        let owner = AccountId::from([7u8; 32]);
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
        let err = node_view::<()>(NodeId(4), Err(rt::pallet::Error::NodeNotFound)).unwrap_err();
        assert!(matches!(err, Error::NodeNotFound(NodeId(4))));

        let err = node_view::<()>(NodeId(4), Err(rt::pallet::Error::ScopeNotFound)).unwrap_err();
        assert!(matches!(err, Error::Runtime(_)));
    }

    #[test]
    fn address_errors_keep_their_meaning() {
        let account = AccountId::from([1u8; 32]);
        let err =
            address_view::<()>(account, Err(rt::AddressViewError::IdentityNotFound)).unwrap_err();
        assert!(matches!(err, Error::IdentityNotFound(a) if a == account));

        let err = address_view::<()>(account, Err(rt::AddressViewError::MissingProjectedIdentity))
            .unwrap_err();
        assert!(matches!(err, Error::MissingProjectedIdentity));
    }
}
