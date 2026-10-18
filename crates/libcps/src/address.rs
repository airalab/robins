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
//! Address handles: the identity-account projection of the CPS tree.

use crate::{
    cps::{Cps, CpsAt},
    error::Result,
    read,
    types::{AccountId, Capability, Meta, NodeId, Payload, ScopeId},
};

/// Handle to a node addressed by its identity [`AccountId`].
///
/// Created by [`Cps::address`]. It reads the same tree as [`Node`](crate::Node)
/// but stays in the account address space: [`parent`](Address::parent) and
/// [`children`](Address::children) return accounts, never [`NodeId`]s. An
/// account is a projection, not a canonical identifier; converting to a
/// [`NodeId`] is explicit with [`Address::node_id`].
///
/// The runtime views are strict: a node without identity cannot be shown as
/// an account, so a result that has to contain one fails as a whole with
/// [`Error::MissingProjectedIdentity`](crate::Error::MissingProjectedIdentity), and an
/// account bound to no node fails with
/// [`Error::IdentityNotFound`](crate::Error::IdentityNotFound).
///
/// The handle is read-only: mutations go through [`Node`](crate::Node).
#[derive(Clone, Copy)]
pub struct Address<'a> {
    cps: &'a Cps,
    account: AccountId,
}

impl<'a> Address<'a> {
    pub(crate) fn new(cps: &'a Cps, account: AccountId) -> Self {
        Self { cps, account }
    }

    /// The identity account of the handle.
    pub fn account(&self) -> &AccountId {
        &self.account
    }

    /// Node the account is bound to, `None` if it is the identity of no node.
    pub async fn node_id(&self) -> Result<Option<NodeId>> {
        self.cps
            .finalized()
            .await?
            .address(self.account)
            .node_id()
            .await
    }

    /// Identity of the parent, `None` for a root node.
    pub async fn parent(&self) -> Result<Option<AccountId>> {
        self.cps
            .finalized()
            .await?
            .address(self.account)
            .parent()
            .await
    }

    /// Identities of the children in creation order.
    pub async fn children(&self) -> Result<Vec<AccountId>> {
        self.cps
            .finalized()
            .await?
            .address(self.account)
            .children()
            .await
    }

    /// Metadata of the node, `None` if unset.
    pub async fn meta(&self) -> Result<Option<Meta>> {
        self.cps
            .finalized()
            .await?
            .address(self.account)
            .meta()
            .await
    }

    /// Payload of the node, `None` if unset.
    pub async fn payload(&self) -> Result<Option<Payload>> {
        self.cps
            .finalized()
            .await?
            .address(self.account)
            .payload()
            .await
    }

    /// Scope of the node.
    pub async fn scope_id(&self) -> Result<ScopeId> {
        self.cps
            .finalized()
            .await?
            .address(self.account)
            .scope_id()
            .await
    }

    /// Whether `caller` holds `capability` at the node, as decided by the runtime.
    pub async fn has_capability(&self, caller: &AccountId, capability: Capability) -> Result<bool> {
        self.cps
            .finalized()
            .await?
            .address(self.account)
            .has_capability(caller, capability)
            .await
    }
}

/// Read-only handle to an address at a pinned block.
///
/// Created by [`CpsAt::address`]; every read uses the block of its [`CpsAt`].
#[derive(Clone, Copy)]
pub struct AddressAt<'a> {
    at: &'a CpsAt,
    account: AccountId,
}

impl<'a> AddressAt<'a> {
    pub(crate) fn new(at: &'a CpsAt, account: AccountId) -> Self {
        Self { at, account }
    }

    /// The identity account of the handle.
    pub fn account(&self) -> &AccountId {
        &self.account
    }

    /// Node the account is bound to, `None` if it is the identity of no node.
    pub async fn node_id(&self) -> Result<Option<NodeId>> {
        read::node_by_identity(self.at.state(), self.account).await
    }

    /// Identity of the parent, `None` for a root node.
    pub async fn parent(&self) -> Result<Option<AccountId>> {
        read::address_parent(self.at.state(), self.account).await
    }

    /// Identities of the children in creation order.
    pub async fn children(&self) -> Result<Vec<AccountId>> {
        read::address_children(self.at.state(), self.account).await
    }

    /// Metadata of the node, `None` if unset.
    pub async fn meta(&self) -> Result<Option<Meta>> {
        read::address_meta(self.at.state(), self.account).await
    }

    /// Payload of the node, `None` if unset.
    pub async fn payload(&self) -> Result<Option<Payload>> {
        read::address_payload(self.at.state(), self.account).await
    }

    /// Scope of the node.
    pub async fn scope_id(&self) -> Result<ScopeId> {
        read::address_scope(self.at.state(), self.account).await
    }

    /// Whether `caller` holds `capability` at the node, as decided by the runtime.
    pub async fn has_capability(&self, caller: &AccountId, capability: Capability) -> Result<bool> {
        read::address_capability(self.at.state(), *caller, self.account, capability).await
    }
}
